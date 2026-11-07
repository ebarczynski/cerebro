//! Git-based sync for Cerebro vaults.
//!
//! Provides automatic commit, push, and pull for profile vaults
//! using Git as the sync backend.

use std::path::Path;
use std::process::Command;

use anyhow::{Result, Context, anyhow};

#[derive(Debug, Clone)]
pub struct SyncConfig {
    pub method: String,
    pub remote: Option<String>,
    pub branch: Option<String>,
    pub auto_commit: bool,
    pub commit_interval_minutes: u64,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            method: "none".to_string(),
            remote: None,
            branch: Some("main".to_string()),
            auto_commit: false,
            commit_interval_minutes: 5,
        }
    }
}

pub struct GitSync {
    vault_path: std::path::PathBuf,
    config: SyncConfig,
}

impl GitSync {
    pub fn new(vault_path: std::path::PathBuf, config: SyncConfig) -> Self {
        Self { vault_path, config }
    }

    pub fn init(&self) -> Result<()> {
        if self.config.method != "git" {
            return Ok(());
        }

        let git_dir = self.vault_path.join(".git");
        if !git_dir.exists() {
            self.git(&["init"])?;
        }

        // Create .gitignore for cerebro internals
        let gitignore = self.vault_path.join(".gitignore");
        if !gitignore.exists() {
            std::fs::write(&gitignore, "# Cerebro internals\ndb.sqlite\n.cerebro/\n")?;
        }

        if let Some(remote) = &self.config.remote {
            // Check if remote already exists
            let output = self.git_output(&["remote"]);
            if !output?.contains("origin") {
                self.git(&["remote", "add", "origin", remote])?;
            }
        }

        Ok(())
    }

    pub fn status(&self) -> Result<String> {
        if self.config.method != "git" {
            return Ok("Sync is disabled".to_string());
        }
        self.git_output(&["status", "--short"])
            .map(|s| if s.is_empty() { "No changes".to_string() } else { s })
    }

    pub fn commit(&self, message: &str) -> Result<String> {
        if self.config.method != "git" {
            return Ok("Sync is disabled".to_string());
        }
        self.git(&["add", "-A"])?;
        self.git(&["commit", "-m", message])?;
        Ok(format!("Committed: {}", message))
    }

    pub fn push(&self) -> Result<String> {
        if self.config.method != "git" {
            return Ok("Sync is disabled".to_string());
        }
        let branch = self.config.branch.as_deref().unwrap_or("main");
        self.git(&["push", "-u", "origin", branch])?;
        Ok("Pushed to remote".to_string())
    }

    pub fn pull(&self) -> Result<String> {
        if self.config.method != "git" {
            return Ok("Sync is disabled".to_string());
        }
        self.git(&["pull", "origin", self.config.branch.as_deref().unwrap_or("main")])?;
        Ok("Pulled from remote".to_string())
    }

    fn git(&self, args: &[&str]) -> Result<()> {
        let output = Command::new("git")
            .args(args)
            .current_dir(&self.vault_path)
            .output()
            .with_context(|| format!("running git {}", args.join(" ")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow!("git {} failed: {}", args.join(" "), stderr));
        }
        Ok(())
    }

    fn git_output(&self, args: &[&str]) -> Result<String> {
        let output = Command::new("git")
            .args(args)
            .current_dir(&self.vault_path)
            .output()
            .with_context(|| format!("running git {}", args.join(" ")))?;

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
}
