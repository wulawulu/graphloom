//! Test-only projection of GraphLoom's Parquet reader over managed index tables.

use std::{
    collections::BTreeMap,
    env,
    io::{self, Write},
};

use graphloom_storage::{ParquetTableProvider, TableProvider};
use serde::Serialize;

const STANDARD_TABLES: [&str; 7] = [
    "documents",
    "text_units",
    "entities",
    "relationships",
    "covariates",
    "communities",
    "community_reports",
];

#[derive(Debug, Serialize)]
struct TableProjection {
    columns: Vec<String>,
    rows: usize,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = env::args_os().nth(1).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: compat_table_reader <parquet-output-directory>",
        )
    })?;
    let provider = ParquetTableProvider::new(root)?;
    let mut projection = BTreeMap::new();
    for table_name in STANDARD_TABLES {
        let dataframe = provider.read_dataframe(table_name).await?;
        projection.insert(
            table_name,
            TableProjection {
                columns: dataframe
                    .get_column_names()
                    .iter()
                    .map(ToString::to_string)
                    .collect(),
                rows: dataframe.height(),
            },
        );
    }

    let stdout = io::stdout();
    let mut output = stdout.lock();
    serde_json::to_writer(&mut output, &projection)?;
    output.write_all(b"\n")?;
    Ok(())
}
