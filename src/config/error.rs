use std::path::PathBuf;
use thiserror::Error;

/// Errors that can occur during configuration loading, saving, or validation.
#[derive(Error, Debug)]
pub enum ConfigError {
    /// The requested configuration file was not found.
    #[error("config file not found: {path}")]
    FileNotFound { path: PathBuf },

    /// The configuration file contains invalid TOML.
    #[error("failed to parse config file {path}: {source}")]
    ParseError {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },

    /// Failed to serialize the configuration to TOML.
    #[error("failed to serialize config: {0}")]
    SerializeError(#[from] toml::ser::Error),

    /// An I/O error occurred while reading or writing a config file.
    #[error("I/O error for {path}: {source}")]
    IoError {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// The requested profile does not exist.
    #[error("profile not found: {name}")]
    ProfileNotFound { name: String },

    /// The profile name contains invalid characters.
    #[error("invalid profile name: {name}")]
    InvalidProfileName { name: String },

    /// A required configuration value is missing.
    #[error("missing required config value: {field}")]
    MissingField { field: String },

    /// The configuration home directory could not be determined.
    #[error("cannot determine cerebro config directory")]
    NoConfigDir,

    /// A validation error occurred.
    #[error("config validation error: {0}")]
    Validation(String),
}

impl ConfigError {
    /// Helper to attach a path context to an I/O error.
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::IoError {
            path: path.into(),
            source,
        }
    }

    /// Helper to attach a path context to a TOML parse error.
    pub fn parse(path: impl Into<PathBuf>, source: toml::de::Error) -> Self {
        Self::ParseError {
            path: path.into(),
            source,
        }
    }
}

/// Alias for results using [`ConfigError`].
pub type Result<T> = std::result::Result<T, ConfigError>;
