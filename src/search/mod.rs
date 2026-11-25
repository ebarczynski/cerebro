//! Full-text search over the SQLite FTS5 index (ADR 0002).
//!
//! A thin named wrapper around [`Storage::search`]; ranking is FTS5's own.

use anyhow::Result;

use crate::storage::{SearchResult, Storage};

pub struct SearchEngine<'a> {
    storage: &'a Storage,
}

impl<'a> SearchEngine<'a> {
    pub fn new(storage: &'a Storage) -> Self {
        Self { storage }
    }

    pub fn search(&self, query: &str, limit: i64) -> Result<Vec<SearchResult>> {
        let results = self.storage.search(query, limit)?;
        Ok(results)
    }
}
