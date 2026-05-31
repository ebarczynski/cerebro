//! Search engine combining SQLite FTS5 for exact keyword search
//! with configurable hybrid ranking.

use anyhow::Result;
use rusqlite::params;

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
