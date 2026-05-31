//! Knowledge Palace connector.
//!
//! Proxies semantic/RAG search queries to the Knowledge Palace API
//! (FastAPI + pgvector) and caches results locally.

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KpConfig {
    pub enabled: bool,
    pub endpoint: String,
}

impl Default for KpConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: "http://localhost:8080".to_string(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct KpSearchResponse {
    results: Vec<KpSearchResult>,
}

#[derive(Debug, Deserialize)]
struct KpSearchResult {
    content: String,
    score: f64,
    source: String,
    author: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct KpSearchHit {
    pub content: String,
    pub score: f64,
    pub source: String,
    pub author: Option<String>,
}

pub struct KpConnector {
    client: reqwest::Client,
    config: KpConfig,
}

impl KpConnector {
    pub fn new(config: KpConfig) -> Self {
        Self {
            client: reqwest::Client::new(),
            config,
        }
    }

    /// Check if Knowledge Palace is reachable.
    pub async fn health_check(&self) -> Result<bool> {
        if !self.config.enabled {
            return Ok(false);
        }
        let url = format!("{}/api/v1/health", self.config.endpoint);
        match self.client.get(&url).timeout(std::time::Duration::from_secs(3)).send().await {
            Ok(resp) => Ok(resp.status().is_success()),
            Err(_) => Ok(false),
        }
    }

    /// Semantic search via Knowledge Palace.
    pub async fn semantic_search(&self, query: &str, limit: i64) -> Result<Vec<KpSearchHit>> {
        if !self.config.enabled {
            return Err(anyhow!("Knowledge Palace integration is disabled"));
        }

        let url = format!(
            "{}/api/v1/search?q={}&mode=semantic&limit={}",
            self.config.endpoint,
            urlencoding(&query),
            limit
        );

        let resp = self.client
            .get(&url)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await?
            .json::<KpSearchResponse>()
            .await?;

        Ok(resp.results.into_iter().map(|r| KpSearchHit {
            content: r.content,
            score: r.score,
            source: r.source,
            author: r.author,
        }).collect())
    }

    /// Hybrid search (vector + FTS) via Knowledge Palace.
    pub async fn hybrid_search(&self, query: &str, limit: i64) -> Result<Vec<KpSearchHit>> {
        if !self.config.enabled {
            return Err(anyhow!("Knowledge Palace integration is disabled"));
        }

        let url = format!(
            "{}/api/v1/search?q={}&mode=hybrid&limit={}",
            self.config.endpoint,
            urlencoding(&query),
            limit
        );

        let resp = self.client
            .get(&url)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await?
            .json::<KpSearchResponse>()
            .await?;

        Ok(resp.results.into_iter().map(|r| KpSearchHit {
            content: r.content,
            score: r.score,
            source: r.source,
            author: r.author,
        }).collect())
    }

    /// RAG-style ask: returns formatted context passages.
    pub async fn ask(&self, question: &str, limit: i64) -> Result<String> {
        let hits = self.hybrid_search(question, limit).await?;
        let mut context = String::from("Knowledge Palace Results:\n\n");
        for (i, hit) in hits.iter().enumerate() {
            context.push_str(&format!(
                "[{}] {} (score: {:.3}, source: {})\n{}\n\n",
                i + 1,
                hit.author.as_deref().unwrap_or("Unknown"),
                hit.score,
                hit.source,
                hit.content
            ));
        }
        Ok(context)
    }
}

fn urlencoding(s: &str) -> String {
    s.replace(' ', "%20")
        .replace('#', "%23")
        .replace('&', "%26")
        .replace('?', "%3F")
}
