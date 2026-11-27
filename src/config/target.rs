//! Profile targeting shared by every entry point (CLI, MCP, GUI).
//!
//! One rule, in one place: a profile the *user named* — `--profile work`, the
//! MCP `profile` argument, or the one picked by `cerebro profile switch` —
//! must already exist. Only the implicit default profile is created on first
//! use. Otherwise a typo such as `--profile persnoal` looks exactly like an
//! empty vault.

use std::path::PathBuf;

use anyhow::Result;

use super::app::Config;
use super::error::ConfigError;
use super::profile::ProfileConfig;
use crate::crypto::VaultKey;
use crate::storage::Storage;

/// The profile a command acts on.
#[derive(Debug, Clone)]
pub struct ProfileTarget {
    /// Profile name (the directory name under `profiles_dir`).
    pub name: String,
    /// Its directory, holding `vault.toml` and `db.sqlite`.
    pub dir: PathBuf,
}

impl ProfileConfig {
    /// Reject profile names that are empty or could name something other than
    /// a directory under `profiles_dir`.
    pub fn check_name(name: &str) -> std::result::Result<(), ConfigError> {
        if name.is_empty() {
            return Err(ConfigError::Validation("profile name must not be empty".to_string()));
        }
        // Reject names that look like path traversals.
        if name.contains("..") || name.contains('/') || name.contains('\\') {
            return Err(ConfigError::InvalidProfileName { name: name.to_string() });
        }
        Ok(())
    }
}

impl Config {
    /// Work out which profile a command acts on.
    ///
    /// `requested` is a profile named by the caller. Without one, the profile
    /// chosen with `cerebro profile switch` is used, and failing that the
    /// default one. Named profiles must exist; the default profile is created
    /// on first use.
    pub fn profile_target(&self, requested: Option<&str>) -> Result<ProfileTarget> {
        let requested = requested.map(str::trim).filter(|name| !name.is_empty());
        let must_exist = requested.is_some() || self.active_profile.is_some();
        let name = requested
            .map(str::to_string)
            .or_else(|| self.active_profile.clone())
            .unwrap_or_else(|| self.default_profile.clone());

        let dir = self.profiles_dir.join(&name);
        if must_exist {
            ProfileConfig::check_name(&name)?;
            if !ProfileConfig::vault_config_path(&dir).exists() {
                let not_found = ConfigError::ProfileNotFound { name: name.clone() };
                return Err(anyhow::Error::new(not_found)
                    .context(format!("run `cerebro init --name {name}` to create it")));
            }
        }
        Ok(ProfileTarget { name, dir })
    }

    /// Open the index of the profile a command acts on (see [`Self::profile_target`]).
    ///
    /// A protected vault (ADR 0006) is refused — `VAULT LOCKED` — because this
    /// has no key for it; see [`Self::open_storage_unlocked`]. Nothing here
    /// reads or writes such a vault in plaintext.
    pub fn open_storage(&self, requested: Option<&str>) -> Result<Storage> {
        self.open_storage_unlocked(requested, |_| Ok(None))
    }

    /// [`Self::open_storage`] for a protected vault too.
    ///
    /// When the vault is protected, `unlock` is asked for its key: it returns
    /// one, or fails with the reason the vault cannot be unlocked (no
    /// passphrase given, not unlocked for this MCP session, …). It is not
    /// called for a plaintext vault, so nothing ever prompts for a vault that
    /// needs no key.
    pub fn open_storage_unlocked(
        &self,
        requested: Option<&str>,
        unlock: impl FnOnce(&ProfileConfig) -> Result<Option<VaultKey>>,
    ) -> Result<Storage> {
        let target = self.profile_target(requested)?;
        let profile_config = ProfileConfig::load_or_default(&target.dir, &target.name)?;
        let key = if profile_config.encryption {
            unlock(&profile_config)?
        } else {
            None
        };
        let mut storage = Storage::open_with_key(&target.dir, &profile_config, key.as_ref())?;
        // The index is a cache of the `.md` files: if it is gone, rebuild it.
        storage.rebuild_if_empty()?;
        Ok(storage)
    }
}
