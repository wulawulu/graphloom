//! Compile-time Send guarantees for the public Query futures.

#![recursion_limit = "256"]

use std::path::PathBuf;

use graphloom::{
    GraphRagConfig,
    api::{basic_search, drift_search, global_search, local_search, query},
    query::{QueryOptions, SearchMethod},
};

fn assert_send_static<T: Send + 'static>(_: T) {}

fn options(method: SearchMethod) -> QueryOptions {
    QueryOptions::new(PathBuf::from("."), "query".to_owned(), method)
}

#[test]
fn test_should_keep_public_query_futures_send_and_static() {
    assert_send_static(query(
        GraphRagConfig::default(),
        options(SearchMethod::Local),
    ));
    assert_send_static(local_search(
        GraphRagConfig::default(),
        options(SearchMethod::Local),
    ));
    assert_send_static(basic_search(
        GraphRagConfig::default(),
        options(SearchMethod::Basic),
    ));
    assert_send_static(global_search(
        GraphRagConfig::default(),
        options(SearchMethod::Global),
    ));
    assert_send_static(drift_search(
        GraphRagConfig::default(),
        options(SearchMethod::Drift),
    ));
}
