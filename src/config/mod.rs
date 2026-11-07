//! Configuration management for Cerebro.
//!
//! This module provides TOML-based configuration for the global app settings
//! and per-profile vault configuration. Configuration files live under
//! `~/.cerebro/`:
//!
//! - `~/.cerebro/config.toml` — global app settings (default profile, editor, etc.)
//! - `~/.cerebro/profiles/<name>/vault.toml` — per-profile vault settings
//!
//! # Example
//!
//! ```no_run
//! use cerebro::config::Config;
//!
//! let cfg = Config::load().unwrap();
//! println!("default profile: {}", cfg.default_profile);
//! ```

pub mod app;
pub mod error;
pub mod profile;

// Public re-exports for ergonomic access.
pub use app::{Config, KnowledgePalaceConfig};
pub use error::{ConfigError, Result};
pub use profile::{KnowledgePalaceSection, ProfileConfig, SyncMethod, SyncSection, VaultKind};
