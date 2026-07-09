//! `graphloom` binary entrypoint.

#![forbid(unsafe_code)]
#![warn(rust_2024_compatibility, missing_docs, missing_debug_implementations)]

use clap::Parser;
use graphloom_cli::{Cli, run};

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    if let Err(error) = run(cli).await {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
