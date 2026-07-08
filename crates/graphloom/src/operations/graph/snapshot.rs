//! Graph snapshot generation operations.

use std::collections::BTreeSet;

use super::FinalRelationshipRow;

pub(crate) fn graphml_snapshot(rows: &[FinalRelationshipRow]) -> String {
    let mut graphml = String::from(
        r#"<?xml version="1.0" encoding="utf-8"?>
<graphml xmlns="http://graphml.graphdrawing.org/xmlns">
<key id="weight" for="edge" attr.name="weight" attr.type="double"/>
<graph edgedefault="undirected">
"#,
    );
    let mut nodes = BTreeSet::new();
    for row in rows {
        nodes.insert(row.source.as_str());
        nodes.insert(row.target.as_str());
    }
    for node in nodes {
        graphml.push_str(&format!(r#"<node id="{}"/>"#, xml_escape(node)));
        graphml.push('\n');
    }
    for row in rows {
        graphml.push_str(&format!(
            r#"<edge source="{}" target="{}"><data key="weight">{}</data></edge>"#,
            xml_escape(&row.source),
            xml_escape(&row.target),
            row.weight,
        ));
        graphml.push('\n');
    }
    graphml.push_str("</graph>\n</graphml>\n");
    graphml
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
