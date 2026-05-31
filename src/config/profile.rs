use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::error::{ConfigError, Result};

/// Filename for per-profile vault configuration.
const VAULT_CONFIG_FILENAME: &str = "vault.toml";

// ---------------------------------------------------------------------------
// Sync method
// ---------------------------------------------------------------------------

/// Sync strategy for a profile vault.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum SyncMethod {
    /// No synchronisation configured.
    #[default]
    None,
    /// Synchronise via git push/pull.
    Git,
}

// ---------------------------------------------------------------------------
// TOML section structs
// ---------------------------------------------------------------------------

/// Knowledge Palace integration settings for a profile.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KnowledgePalaceSection {
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

impl Default for KnowledgePalaceSection {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: String::new(),
            sync_scope: default_sync_scope(),
        }
    }
}

/// Confluence integration settings for a profile.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConfluenceSection {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub space_key: String,
}

impl Default for ConfluenceSection {
    fn default() -> Self {
        Self {
            enabled: false,
            base_url: String::new(),
            space_key: String::new(),
        }
    }
}

/// Sync settings for a profile vault.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SyncSection {
    #[serde(default)]
    pub method: SyncMethod,
    #[serde(default)]
    pub remote: String,
    #[serde(default = "default_branch")]
    pub branch: String,
}

fn default_branch() -> String {
    "main".to_string()
}

impl Default for SyncSection {
    fn default() -> Self {
        Self {
            method: SyncMethod::None,
            remote: String::new(),
            branch: default_branch(),
        }
    }
}

// ---------------------------------------------------------------------------
// ProfileConfig
// ---------------------------------------------------------------------------

/// Per-profile vault configuration.
///
/// Each profile stores its own `vault.toml` under `~/.cerebro/profiles/<name>/`.
/// The `notes_dir` is resolved relative to the profile directory unless an
/// absolute path is given.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProfileConfig {
    /// Machine-readable profile identifier (directory name).
    pub name: String,

    /// Human-friendly display name.
    #[serde(default)]
    pub display_name: String,

    /// Path to the notes directory — relative to the profile dir unless
    /// absolute.
    #[serde(default = "default_notes_dir")]
    pub notes_dir: PathBuf,

    /// Whether the vault contents are encrypted at rest.
    #[serde(default)]
    pub encryption: bool,

    /// Knowledge Palace integration.
    #[serde(default)]
    pub knowledge_palace: KnowledgePalaceSection,

    /// Confluence integration.
    #[serde(default)]
    pub confluence: ConfluenceSection,

    /// Synchronisation settings.
    #[serde(default)]
    pub sync: SyncSection,
}

fn default_notes_dir() -> PathBuf {
    PathBuf::from("notes")
}

// ---------------------------------------------------------------------------
// Constructors
// ---------------------------------------------------------------------------

impl ProfileConfig {
    /// Create a new profile config with the given name.
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();
        let display_name = name.clone();
        Self {
            name,
            display_name,
            notes_dir: default_notes_dir(),
            encryption: false,
            knowledge_palace: KnowledgePalaceSection::default(),
            confluence: ConfluenceSection::default(),
            sync: SyncSection::default(),
        }
    }

    /// Return the path to the `vault.toml` for a profile stored in `profile_dir`.
    pub fn vault_config_path(profile_dir: &Path) -> PathBuf {
        profile_dir.join(VAULT_CONFIG_FILENAME)
    }

    /// Return the absolute notes directory, resolving relative paths against
    /// `profile_dir`.
    pub fn resolved_notes_dir(&self, profile_dir: &Path) -> PathBuf {
        if self.notes_dir.is_absolute() {
            self.notes_dir.clone()
        } else {
            profile_dir.join(&self.notes_dir)
        }
    }

    /// Load a profile config from its directory.
    ///
    /// The directory is expected to contain a `vault.toml` file. If the file
    /// does not exist a default config is created, saved, and returned.
    pub fn load(profile_dir: &Path) -> Result<Self> {
        let path = Self::vault_config_path(profile_dir);
        if !path.exists() {
            // Derive profile name from the directory name.
            let name = profile_dir
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "default".to_string());

            let cfg = Self::new(name);
            cfg.save_to(profile_dir)?;
            return Ok(cfg);
        }

        let contents =
            fs::read_to_string(&path).map_err(|e| ConfigError::io(&path, e))?;

        let cfg: Self = toml::from_str(&contents)
            .map_err(|e| ConfigError::parse(&path, e))?;

        Ok(cfg)
    }

    /// Save the profile config into `profile_dir`, creating the directory
    /// if it does not exist.
    pub fn save_to(&self, profile_dir: &Path) -> Result<()> {
        if !profile_dir.exists() {
            fs::create_dir_all(profile_dir)
                .map_err(|e| ConfigError::io(profile_dir, e))?;
        }

        let path = Self::vault_config_path(profile_dir);
        let toml_str = toml::to_string_pretty(self)?;
        fs::write(&path, toml_str).map_err(|e| ConfigError::io(&path, e))?;

        // Ensure the notes directory also exists.
        let notes = self.resolved_notes_dir(profile_dir);
        if !notes.exists() {
            fs::create_dir_all(&notes)
                .map_err(|e| ConfigError::io(&notes, e))?;
        }

        Ok(())
    }

    /// Save to profile directory (convenience alias).
    pub fn save(&self, profile_dir: &Path) -> Result<()> {
        self.save_to(profile_dir)
    }

    /// Load or create a default config (convenience wrapper).
    pub fn load_or_default(profile_dir: &Path, name: &str) -> Result<Self> {
        let path = Self::vault_config_path(profile_dir);
        if path.exists() {
            let contents = fs::read_to_string(&path).map_err(|e| ConfigError::io(&path, e))?;
            let cfg: Self = toml::from_str(&contents).map_err(|e| ConfigError::parse(&path, e))?;
            Ok(cfg)
        } else {
            let cfg = Self::new(name);
            cfg.save_to(profile_dir)?;
            Ok(cfg)
        }
    }

    /// Validate the profile configuration.
    pub fn validate(&self) -> std::result::Result<(), ConfigError> {
        if self.name.is_empty() {
            return Err(ConfigError::Validation(
                "profile name must not be empty".to_string(),
            ));
        }

        // Reject names that look like path traversals.
        if self.name.contains("..") || self.name.contains('/') || self.name.contains('\\') {
            return Err(ConfigError::InvalidProfileName {
                name: self.name.clone(),
            });
        }

        if self.display_name.is_empty() {
            return Err(ConfigError::Validation(
                "display_name must not be empty".to_string(),
            ));
        }

        // If git sync is enabled, remote must be set.
        if self.sync.method == SyncMethod::Git && self.sync.remote.is_empty() {
            return Err(ConfigError::Validation(
                "git sync requires a remote URL".to_string(),
            ));
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_profile_has_expected_defaults() {
        let p = ProfileConfig::new("work");
        assert_eq!(p.name, "work");
        assert_eq!(p.display_name, "work");
        assert_eq!(p.notes_dir, PathBuf::from("notes"));
        assert!(!p.encryption);
        assert_eq!(p.sync.method, SyncMethod::None);
    }

    #[test]
    fn roundtrip_through_toml() {
        let mut p = ProfileConfig::new("personal");
        p.display_name = "My Personal Vault".to_string();
        p.sync.method = SyncMethod::Git;
        p.sync.remote = "git@github.com:user/notes.git".to_string();

        let toml_str = toml::to_string_pretty(&p).unwrap();
        let parsed: ProfileConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(p, parsed);
    }

    #[test]
    fn load_creates_vault_toml_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let profile_dir = tmp.path().join("personal");
        // Directory does not need to exist – save_to creates it.
        let cfg = ProfileConfig::load(&profile_dir).unwrap();
        assert_eq!(cfg.name, "personal");
        assert!(profile_dir.join("vault.toml").exists());
        assert!(profile_dir.join("notes").exists());
    }

    #[test]
    fn validate_rejects_traversal_name() {
        let p = ProfileConfig::new("../etc");
        assert!(matches!(
            p.validate(),
            Err(ConfigError::InvalidProfileName { .. })
        ));
    }

    #[test]
    fn validate_rejects_git_without_remote() {
        let mut p = ProfileConfig::new("test");
        p.sync.method = SyncMethod::Git;
        assert!(p.validate().is_err());
    }

    #[test]
    fn resolved_notes_dir_absolute() {
        let p = ProfileConfig::new("test");
        let abs = PathBuf::from("/tmp/abs-notes");
        let mut q = p.clone();
        q.notes_dir = abs.clone();
        assert_eq!(
            q.resolved_notes_dir(Path::new("/home/user/.cerebro/profiles/test")),
            abs
        );
    }

    #[test]
    fn resolved_notes_dir_relative() {
        let p = ProfileConfig::new("test");
        assert_eq!(
            p.resolved_notes_dir(Path::new("/home/user/.cerebro/profiles/test")),
            PathBuf::from("/home/user/.cerebro/profiles/test/notes")
        );
    }
}
