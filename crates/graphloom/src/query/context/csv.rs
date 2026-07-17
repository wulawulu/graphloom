//! Deterministic pandas-compatible context table rendering.

use std::borrow::Cow;

use polars_core::prelude::{Column, DataFrame, NamedFrom, Series};

use super::super::{QueryError, Result, SearchMethod};

/// One string-valued context table in stable column and row order.
#[derive(Debug, Clone, Default)]
pub(crate) struct ContextTable {
    columns: Vec<String>,
    rows: Vec<Vec<String>>,
}

impl ContextTable {
    pub(crate) fn new(
        columns: impl IntoIterator<Item = impl Into<String>>,
        rows: Vec<Vec<String>>,
    ) -> Self {
        Self {
            columns: columns.into_iter().map(Into::into).collect(),
            rows,
        }
    }

    pub(crate) fn push(&mut self, row: Vec<String>) {
        self.rows.push(row);
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub(crate) fn render_csv(
        &self,
        method: SearchMethod,
        operation: &'static str,
    ) -> Result<String> {
        self.validate_rows(method, operation)?;
        render_records(
            self.columns.iter().map(String::as_str),
            self.rows.iter().map(|row| row.iter().map(String::as_str)),
            method,
            operation,
        )
    }

    pub(crate) fn render_csv_section(
        &self,
        context_name: &str,
        method: SearchMethod,
        operation: &'static str,
    ) -> Result<String> {
        Ok(format!(
            "-----{context_name}-----\n{}",
            self.render_csv(method, operation)?
        ))
    }

    pub(crate) fn render_csv_header(
        &self,
        context_name: &str,
        method: SearchMethod,
        operation: &'static str,
    ) -> Result<String> {
        self.validate_rows(method, operation)?;
        Ok(format!(
            "-----{context_name}-----\n{}",
            render_record(self.columns.iter().map(String::as_str), method, operation,)?
        ))
    }

    pub(crate) fn render_csv_row(
        &self,
        row: &[String],
        method: SearchMethod,
        operation: &'static str,
    ) -> Result<String> {
        self.validate_row(row, method, operation)?;
        render_record(row.iter().map(String::as_str), method, operation)
    }

    pub(crate) fn render_delimited(
        &self,
        method: SearchMethod,
        operation: &'static str,
    ) -> Result<String> {
        self.validate_rows(method, operation)?;
        let mut rendered = delimited_record(&self.columns);
        for row in &self.rows {
            rendered.push_str(&delimited_record(row));
        }
        Ok(rendered)
    }

    pub(crate) fn render_delimited_section(
        &self,
        context_name: &str,
        method: SearchMethod,
        operation: &'static str,
    ) -> Result<String> {
        Ok(format!(
            "-----{context_name}-----\n{}",
            self.render_delimited(method, operation)?
        ))
    }

    pub(crate) fn render_delimited_header(
        &self,
        context_name: &str,
        method: SearchMethod,
        operation: &'static str,
    ) -> Result<String> {
        self.validate_rows(method, operation)?;
        Ok(format!(
            "-----{context_name}-----\n{}",
            delimited_record(&self.columns)
        ))
    }

    pub(crate) fn render_delimited_row(
        &self,
        row: &[String],
        method: SearchMethod,
        operation: &'static str,
    ) -> Result<String> {
        self.validate_row(row, method, operation)?;
        Ok(delimited_record(row))
    }

    pub(crate) fn to_dataframe(
        &self,
        method: SearchMethod,
        operation: &'static str,
    ) -> Result<DataFrame> {
        self.validate_rows(method, operation)?;
        if self.rows.is_empty() {
            return Ok(DataFrame::empty());
        }
        let mut columns = Vec::<Column>::with_capacity(self.columns.len());
        for (index, name) in self.columns.iter().enumerate() {
            let values = self
                .rows
                .iter()
                .map(|row| {
                    row.get(index)
                        .cloned()
                        .ok_or_else(|| QueryError::QueryContext {
                            method,
                            operation,
                            message: format!(
                                "context row has {} fields but {} columns were declared",
                                row.len(),
                                self.columns.len()
                            ),
                        })
                })
                .collect::<Result<Vec<_>>>()?;
            columns.push(Series::new(name.as_str().into(), values).into());
        }
        DataFrame::new(self.rows.len(), columns).map_err(|source| QueryError::QueryContext {
            method,
            operation,
            message: source.to_string(),
        })
    }

    fn validate_rows(&self, method: SearchMethod, operation: &'static str) -> Result<()> {
        for row in &self.rows {
            self.validate_row(row, method, operation)?;
        }
        Ok(())
    }

    fn validate_row(
        &self,
        row: &[String],
        method: SearchMethod,
        operation: &'static str,
    ) -> Result<()> {
        if row.len() == self.columns.len() {
            return Ok(());
        }
        Err(QueryError::QueryContext {
            method,
            operation,
            message: format!(
                "context row has {} fields but {} columns were declared",
                row.len(),
                self.columns.len()
            ),
        })
    }
}

fn delimited_record(fields: &[String]) -> String {
    let mut rendered = fields.join("|");
    rendered.push('\n');
    rendered
}

fn render_records<'a, H, R, I>(
    header: H,
    rows: R,
    method: SearchMethod,
    operation: &'static str,
) -> Result<String>
where
    H: IntoIterator<Item = &'a str>,
    R: IntoIterator<Item = I>,
    I: IntoIterator<Item = &'a str>,
{
    let mut writer = writer();
    write_record(&mut writer, header, method, operation)?;
    for row in rows {
        write_record(&mut writer, row, method, operation)?;
    }
    finish(writer, method, operation)
}

fn render_record<'a>(
    fields: impl IntoIterator<Item = &'a str>,
    method: SearchMethod,
    operation: &'static str,
) -> Result<String> {
    let mut writer = writer();
    write_record(&mut writer, fields, method, operation)?;
    finish(writer, method, operation)
}

fn write_record<'a>(
    writer: &mut csv::Writer<Vec<u8>>,
    fields: impl IntoIterator<Item = &'a str>,
    method: SearchMethod,
    operation: &'static str,
) -> Result<()> {
    let escaped = fields
        .into_iter()
        .map(pandas_escape_field)
        .collect::<Vec<_>>();
    writer
        .write_record(escaped.iter().map(|field| field.as_bytes()))
        .map_err(|source| csv_error(method, operation, &source))
}

fn writer() -> csv::Writer<Vec<u8>> {
    csv::WriterBuilder::new()
        .delimiter(b'|')
        .escape(b'\\')
        .from_writer(Vec::new())
}

fn finish(
    writer: csv::Writer<Vec<u8>>,
    method: SearchMethod,
    operation: &'static str,
) -> Result<String> {
    let bytes = writer
        .into_inner()
        .map_err(|source| csv_error(method, operation, &source.into_error().into()))?;
    String::from_utf8(bytes).map_err(|source| QueryError::QueryContext {
        method,
        operation,
        message: source.to_string(),
    })
}

fn pandas_escape_field(value: &str) -> Cow<'_, str> {
    if value.contains('\\') {
        Cow::Owned(value.replace('\\', "\\\\"))
    } else {
        Cow::Borrowed(value)
    }
}

fn csv_error(method: SearchMethod, operation: &'static str, source: &csv::Error) -> QueryError {
    QueryError::QueryContext {
        method,
        operation,
        message: source.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::ContextTable;
    use crate::query::{QueryError, SearchMethod};

    #[test]
    fn test_should_render_raw_delimited_fields_without_normalization() {
        let table = ContextTable::new(
            ["id", "text"],
            vec![vec![
                "0".to_owned(),
                "value|\"quoted\" \\path\nline\r\nnext".to_owned(),
            ]],
        );

        assert_eq!(
            table
                .render_delimited_section("Raw", SearchMethod::Local, "render raw test")
                .expect("raw context"),
            "-----Raw-----\nid|text\n0|value|\"quoted\" \\path\nline\r\nnext\n"
        );
        assert!(
            table
                .render_csv_section("Csv", SearchMethod::Global, "render csv test")
                .expect("CSV context")
                .contains("\"value|\"\"quoted\"\" \\\\path")
        );
    }

    #[test]
    fn test_should_reject_delimited_row_with_wrong_field_count() {
        let table = ContextTable::new(["id", "text"], Vec::new());

        assert!(matches!(
            table.render_delimited_row(
                &["only-id".to_owned()],
                SearchMethod::Local,
                "render malformed raw row",
            ),
            Err(QueryError::QueryContext {
                operation: "render malformed raw row",
                ..
            })
        ));
    }
}
