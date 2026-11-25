//! Cerebro — A fast, secure, local-first personal knowledge base.
//!
//! Dual-profile vaults, markdown notes, bidirectional link graph,
//! full-text search, Knowledge Palace connector, and MCP server.

use clap::Parser;

fn main() {
    // Initialize tracing — on STDERR. `tracing_subscriber::fmt()` writes to
    // stdout by default, and in `cerebro mcp` stdout is the JSON-RPC stream:
    // a single log line there corrupts the protocol. With the subscriber on
    // stderr, RUST_LOG=trace is safe in every mode.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let cli = cerebro::cli::Cli::parse();

    if let Err(e) = cerebro::cli::run(cli) {
        // Errors go to stderr: stdout is reserved for command output — and,
        // for `cerebro mcp`, for the JSON-RPC stream.
        eprintln!("Error: {:#}", e);
        std::process::exit(1);
    }
}
