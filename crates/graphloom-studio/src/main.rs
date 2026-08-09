//! `GraphLoom` Studio executable host.

use std::{error::Error, fmt, net::SocketAddr, path::PathBuf};

use clap::Parser;
use graphloom_studio::server::{StudioServerOptions, serve};

struct StudioMainError;

impl fmt::Debug for StudioMainError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("GraphLoom Studio startup or serving failure")
    }
}

impl fmt::Display for StudioMainError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("GraphLoom Studio startup or serving failure")
    }
}

impl Error for StudioMainError {}

#[derive(Debug, Parser)]
#[command(
    name = "graphloom-studio",
    about = "Serve the trusted/local GraphLoom Studio MVP"
)]
struct Cli {
    /// `GraphLoom` project root or concrete settings file.
    #[arg(long, default_value = ".")]
    root: PathBuf,
    /// Listen address. Non-loopback binding exposes an API with no authentication.
    #[arg(long, default_value = "127.0.0.1:8080")]
    listen: SocketAddr,
    /// Optional Vite dist directory. Omit for API-only development mode.
    #[arg(long)]
    assets_dir: Option<PathBuf>,
    /// `SQLite` path; relative values are resolved from the project root.
    #[arg(long)]
    explainability_db: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<(), StudioMainError> {
    let cli = Cli::parse();
    let mut options = StudioServerOptions::new(cli.root).with_listen(cli.listen);
    if let Some(assets_dir) = cli.assets_dir {
        options = options.with_assets_dir(assets_dir);
    }
    if let Some(database) = cli.explainability_db {
        options = options.with_explainability_db(database);
    }
    serve(options).await.map_err(|_error| StudioMainError)
}
