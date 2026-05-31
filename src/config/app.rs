use std::env;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::error::{ConfigError, Result};

/// Filename for the global Cerebro configuration.
const CONFIG_FILENAME: &str = "config.toml";

/// Directory name under the config home where Cerebro stores its data.
const CEREBRO_DIR: &str = ".cerebro";

/// Default profiles created on first run.
const DEFAULT_PROFILES: &[&str] = &["personal", "professional"];

// ---------------------------------------------------------------------------
// Top-level TOML sections
// ---------------------------------------------------------------------------

/// Knowledge Palace integration settings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KnowledgePalaceConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub endpoint: String,
    #[serde(default = "default_sync_scope")]
    pub sync_scope: String,
}

fn default_sync_scope() -> String {
    "all".to_string()
}

impl Default for KnowledgePalaceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: String::new(),
            sync_scope: default_sync_scope(),
        }
    }
}

/// Confluence integration settings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConfluenceConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub space_key: String,
}

impl Default for ConfluenceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            base_url: String::new(),
            space_key: String::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Global Config struct
// ---------------------------------------------------------------------------

/// Global Cerebro application configuration.
///
/// Stored at `~/.cerebro/config.toml`. Manages the default profile, editor
/// preference, and the directory where all profile vaults live.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    /// Name of the profile used when none is explicitly specified.
    #[serde(default = "default_profile")]
    pub default_profile: String,

    /// Editor command used to open notes (e.g. "vim", "code", "nano").
    #[serde(default = "default_editor")]
    pub editor: String,

    /// Root directory for all profile vaults.
    #[serde(default = "default_profiles_dir")]
    pub profiles_dir: PathBuf,

    /// Currently active profile name, if one has been selected.
    #[serde(default)]
    pub active_profile: Option<String>,

    /// Knowledge Palace integration settings.
    #[serde(default)]
    pub knowledge_palace: KnowledgePalaceConfig,

    /// Confluence integration settings.
    #[serde(default)]
    pub confluence: ConfluenceConfig,
}

fn default_profile() -> String {
    "personal".to_string()
}

fn default_editor() -> String {
    env::var("EDITOR").unwrap_or_else(|_| "vi".to_string())
}

fn default_profiles_dir() -> PathBuf {
    config_home()
        .expect("unable to determine config home directory")
        .join("profiles")
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Return the Cerebro configuration home directory (`~/.cerebro`).
///
/// Creates the directory if it does not already exist.
pub fn config_home() -> Result<PathBuf> {
    let base = dirs::home_dir().ok_or(ConfigError::NoConfigDir)?;
    let dir = base.join(CEREBRO_DIR);
    if !dir.exists() {
        fs::create_dir_all(&dir).map_err(|e| ConfigError::io(&dir, e))?;
    }
    Ok(dir)
}

/// Return the path to the global config file.
pub fn config_path() -> Result<PathBuf> {
    Ok(config_home()?.join(CONFIG_FILENAME))
}

// ---------------------------------------------------------------------------
// Constructor / Load / Save
// ---------------------------------------------------------------------------

impl Config {
    /// Create a new `Config` with sensible defaults.
    pub fn new() -> Result<Self> {
        Ok(Self {
            default_profile: default_profile(),
            editor: default_editor(),
            profiles_dir: default_profiles_dir(),
            active_profile: None,
            knowledge_palace: KnowledgePalaceConfig::default(),
            confluence: ConfluenceConfig::default(),
        })
    }

    /// Load the global configuration from disk.
    ///
    /// If the file does not exist a default configuration is created and
    /// persisted so that subsequent calls will find it.
    pub fn load() -> Result<Self> {
        let path = config_path()?;
        if !path.exists() {
            let cfg = Self::new()?;
            cfg.save()?;
            return Ok(cfg);
        }

        let contents =
            fs::read_to_string(&path).map_err(|e| ConfigError::io(&path, e))?;

        let cfg: Self = toml::from_str(&contents)
            .map_err(|e| ConfigError::parse(&path, e))?;

        Ok(cfg)
    }

    /// Save the configuration to disk, creating parent directories as needed.
    pub fn save(&self) -> Result<()> {
        let path = config_path()?;
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)
                    .map_err(|e| ConfigError::io(parent, e))?;
            }
        }

        let toml_str = toml::to_string_pretty(self)?;
        fs::write(&path, toml_str).map_err(|e| ConfigError::io(&path, e))?;

        Ok(())
    }

    /// Return the full path for a named profile directory.
    pub fn profile_dir(&self, name: &str) -> PathBuf {
        self.profiles_dir.join(name)
    }

    /// Ensure that the default profile directories exist on disk.
    ///
    /// Called on first run so that users find a ready-to-use directory
    /// structure under `~/.cerebro/profiles/`.
    pub fn ensure_default_profiles(&self) -> Result<()> {
        for name in DEFAULT_PROFILES {
            let dir = self.profile_dir(name);
            if !dir.exists() {
                fs::create_dir_all(&dir)
                    .map_err(|e| ConfigError::io(&dir, e))?;
            }
        }
        Ok(())
    }

    /// Load config, creating defaults if missing (convenience wrapper).
    pub fn load_or_create() -> Result<Self> {
        Self::load()
    }

    /// Load config from a specific path.
    pub fn load_from(path: &std::path::Path) -> Result<Self> {
        if !path.exists() {
            let cfg = Self::new()?;
            cfg.save_to(path)?;
            return Ok(cfg);
        }
        let contents = fs::read_to_string(path).map_err(|e| ConfigError::io(path, e))?;
        let cfg: Self = toml::from_str(&contents).map_err(|e| ConfigError::parse(path, e))?;
        Ok(cfg)
    }

    /// Save config to a specific path.
    pub fn save_to(&self, path: &std::path::Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent).map_err(|e| ConfigError::io(parent, e))?;
            }
        }
        let toml_str = toml::to_string_pretty(self)?;
        fs::write(path, toml_str).map_err(|e| ConfigError::io(path, e))?;
        Ok(())
    }

    /// Validate the configuration, returning an error description if invalid.
    pub fn validate(&self) -> std::result::Result<(), ConfigError> {
        if self.default_profile.is_empty() {
            return Err(ConfigError::Validation(
                "default_profile must not be empty".to_string(),
            ));
        }
        if self.profiles_dir.as_os_str().is_empty() {
            return Err(ConfigError::Validation(
                "profiles_dir must not be empty".to_string(),
            ));
        }
        Ok(())
    }
}

impl Default for Config {
    fn default() -> Self {
        // Default construction should not fail – we only call this in tests
        // or when $HOME is available.
        Self::new().expect("failed to build default Config")
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn default_config_roundtrips_through_toml() {
        let cfg = Config::default();
        let toml_str = toml::to_string_pretty(&cfg).unwrap();
        let parsed: Config = toml::from_str(&toml_str).unwrap();
        assert_eq!(cfg, parsed);
    }

    #[test]
    fn load_creates_config_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        // Override config_home by setting HOME (dirs crate reads $HOME on linux).
        let orig_home = env::var("HOME").ok();
        // SAFETY: test-only, single-threaded
        unsafe { env::set_var("HOME", tmp.path()); }
        // Ensure clean state – remove any leftover .cerebro from temp.
        let test_cfg_dir = tmp.path().join(CEREBRO_DIR);
        let _ = fs::remove_dir_all(&test_cfg_dir);

        // Now config_home() will resolve to tmp/.cerebro.
        // We need to call load() which uses config_path -> config_home.
        let cfg = Config::load().unwrap();
        assert!(cfg.default_profile == "personal");
        assert!(test_cfg_dir.join(CONFIG_FILENAME).exists());

        // Restore HOME.
        // SAFETY: test-only, single-threaded
        match orig_home {
            Some(h) => unsafe { env::set_var("HOME", h); }
            None => unsafe { env::remove_var("HOME"); }
        }
    }

    #[test]
    fn validate_rejects_empty_default_profile() {
        let mut cfg = Config::default();
        cfg.default_profile = String::new();
        assert!(cfg.validate().is_err());
    }
}
