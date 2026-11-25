//! Git-based sync for Cerebro vaults.
//!
//! Wraps the `git` command-line tool (there is no git library in the
//! dependency tree) to commit, push and pull a vault's notes directory.
//! Sync is **manual**: nothing here runs on a timer or on its own — every
//! entry point is the `cerebro sync status|push|pull` command.
//!
//! Conflicts are left to git: a `pull` that conflicts fails with git's own
//! message and leaves the vault exactly as git would (conflict markers in
//! the affected `.md` files, `MERGE_HEAD` present). Resolve them with your
//! usual git tools, then `cerebro reindex`.

use std::path::PathBuf;
use std::process::Command;

use anyhow::{anyhow, Context, Result};

#[derive(Debug, Clone)]
pub struct SyncConfig {
    pub method: String,
    pub remote: Option<String>,
    pub branch: Option<String>,
    /// Kept for config compatibility. Nothing is committed automatically.
    pub auto_commit: bool,
    /// Kept for config compatibility. Nothing is committed automatically.
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
    vault_path: PathBuf,
    config: SyncConfig,
}

impl GitSync {
    pub fn new(vault_path: PathBuf, config: SyncConfig) -> Self {
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
        // The repository's branch is the configured one from the very first
        // commit, whatever the machine's `init.defaultBranch` says — `push`
        // targets the configured branch.
        if let Some(branch) = &self.config.branch {
            self.git(&["symbolic-ref", "HEAD", &format!("refs/heads/{branch}")])?;
        }

        // Create .gitignore for cerebro internals
        let gitignore = self.vault_path.join(".gitignore");
        if !gitignore.exists() {
            std::fs::write(&gitignore, "# Cerebro internals\ndb.sqlite\n.cerebro/\n")?;
        }

        if let Some(remote) = &self.config.remote {
            // Check if remote already exists
            let output = self.git_output(&["remote"])?;
            if !output.contains("origin") {
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

    /// Commit everything that changed. A vault with nothing to commit is not
    /// an error — there may still be earlier commits to push.
    pub fn commit(&self, message: &str) -> Result<String> {
        if self.config.method != "git" {
            return Ok("Sync is disabled".to_string());
        }
        if self.git_output(&["status", "--porcelain"])?.is_empty() {
            return Ok("Nothing to commit".to_string());
        }
        self.git(&["add", "-A"])?;
        self.git(&["commit", "-m", message])?;
        Ok(format!("Committed: {}", message))
    }

    pub fn push(&self) -> Result<String> {
        if self.config.method != "git" {
            return Ok("Sync is disabled".to_string());
        }
        // Nothing has ever been committed: there is no branch to push yet.
        if self.git(&["rev-parse", "--verify", "HEAD"]).is_err() {
            return Ok("Nothing to push yet".to_string());
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
            return Err(anyhow!("git {} failed: {}", args.join(" "), stderr.trim()));
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
