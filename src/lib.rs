//! Cerebro — A fast, secure, local-first personal knowledge base.
//!
//! This library provides the core types and configuration for Cerebro,
//! including dual-profile vault management, full-text search, Markdown
//! parsing, bidirectional links, and MCP integration.

pub mod cli;
pub mod config;
pub mod import;
pub mod kp;
pub mod links;
pub mod mcp;
pub mod parser;
pub mod publish;
pub mod search;
pub mod storage;
pub mod sync;
