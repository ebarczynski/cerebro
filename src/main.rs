//! Cerebro — A fast, secure, local-first personal knowledge base.
//!
//! Dual-profile vaults, markdown notes, bidirectional link graph,
//! full-text search, Knowledge Palace connector, and MCP server.

use clap::Parser;

fn main() {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let cli = cerebro::cli::Cli::parse();

    if let Err(e) = cerebro::cli::run(cli) {
        eprintln!("Error: {:#}", e);
        std::process::exit(1);
    }
}
