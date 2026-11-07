//! Knowledge Palace connector.
//!
//! Proxies semantic/RAG search queries to the Knowledge Palace API
//! (FastAPI + pgvector).
//!
//! # Wire contract (VERIFIED)
//!
//! Verified against KP source at commit `82775ae` on 2026-11-07
//! (`src/knowledge_palace/api/app.py`, `src/knowledge_palace/api/models.py`):
//!
//! - `GET /api/v1/health` → `{"status": "ok"}`
//! - `GET /api/v1/search?q=…&mode=semantic|keyword|hybrid&limit=1..100`
//!   (optional `source`, `author` filters) →
//!   `{"total", "mode", "query", "results": [{chunk_id, document_id, content,
//!   score, title, author, source, tags, highlights, provenance}]}`
//!
//! Query parameters are built with reqwest's `.query()`, which percent-encodes
//! properly — the former hand-rolled 4-character encoder broke on `%`, quotes
//! and non-ASCII (grill finding F11).

use std::time::Duration;

use anyhow::{anyhow, Result};
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

/// Search modes accepted by KP's `/api/v1/search` (verified).
pub const KP_MODES: &[&str] = &["semantic", "keyword", "hybrid"];

#[derive(Debug, Deserialize)]
struct KpSearchResponse {
    #[serde(default)]
    total: u64,
    #[serde(default)]
    mode: String,
    #[serde(default)]
    query: String,
    results: Vec<KpSearchResultItem>,
}

#[derive(Debug, Deserialize)]
struct KpSearchResultItem {
    #[serde(default)]
    chunk_id: String,
    #[serde(default)]
    document_id: String,
    content: String,
    score: f64,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    author: Option<String>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KpSearchHit {
    pub chunk_id: String,
    pub document_id: String,
    pub content: String,
    pub score: f64,
    pub title: Option<String>,
    pub author: Option<String>,
    pub source: Option<String>,
    pub tags: Vec<String>,
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

    fn search_url(&self) -> String {
        format!(
            "{}/api/v1/search",
            self.config.endpoint.trim_end_matches('/')
        )
    }

    /// Check if Knowledge Palace is reachable and reports healthy.
    pub async fn health_check(&self) -> Result<bool> {
        if !self.config.enabled {
            return Ok(false);
        }
        let url = format!(
            "{}/api/v1/health",
            self.config.endpoint.trim_end_matches('/')
        );
        let resp = self
            .client
            .get(&url)
            .timeout(Duration::from_secs(3))
            .send()
            .await?;
        if !resp.status().is_success() {
            return Ok(false);
        }
        // Verified body shape: {"status": "ok"}
        let body: serde_json::Value = resp.json().await?;
        Ok(body.get("status").and_then(|s| s.as_str()) == Some("ok"))
    }

    /// Search with an explicit KP mode ("semantic" | "keyword" | "hybrid").
    pub async fn search_with_mode(
        &self,
        query: &str,
        mode: &str,
        limit: i64,
    ) -> Result<Vec<KpSearchHit>> {
        if !self.config.enabled {
            return Err(anyhow!("Knowledge Palace integration is disabled"));
        }
        if !KP_MODES.contains(&mode) {
            return Err(anyhow!(
                "invalid KP mode '{}'; expected one of: {}",
                mode,
                KP_MODES.join(", ")
            ));
        }

        // .query() percent-encodes every parameter value (spaces, quotes,
        // '%', '&', non-ASCII) — never build the query string by hand.
        let limit_str = limit.to_string();
        let resp = self
            .client
            .get(self.search_url())
            .query(&[("q", query), ("mode", mode), ("limit", limit_str.as_str())])
            .timeout(Duration::from_secs(10))
            .send()
            .await?
            .error_for_status()?
            .json::<KpSearchResponse>()
            .await?;

        Ok(resp
            .results
            .into_iter()
            .map(|r| KpSearchHit {
                chunk_id: r.chunk_id,
                document_id: r.document_id,
                content: r.content,
                score: r.score,
                title: r.title,
                author: r.author,
                source: r.source,
                tags: r.tags,
            })
            .collect())
    }

    /// Semantic search via Knowledge Palace.
    pub async fn semantic_search(&self, query: &str, limit: i64) -> Result<Vec<KpSearchHit>> {
        self.search_with_mode(query, "semantic", limit).await
    }

    /// Hybrid search (vector + FTS) via Knowledge Palace.
    pub async fn hybrid_search(&self, query: &str, limit: i64) -> Result<Vec<KpSearchHit>> {
        self.search_with_mode(query, "hybrid", limit).await
    }

    /// RAG-style ask: returns formatted context passages.
    pub async fn ask(&self, question: &str, limit: i64) -> Result<String> {
        let hits = self.hybrid_search(question, limit).await?;
        let mut context = String::from("Knowledge Palace Results:\n\n");
        for (i, hit) in hits.iter().enumerate() {
            context.push_str(&format!(
                "[{}] {} (score: {:.3}, source: {})\n{}\n\n",
                i + 1,
                hit.title
                    .as_deref()
                    .or(hit.author.as_deref())
                    .unwrap_or("Unknown"),
                hit.score,
                hit.source.as_deref().unwrap_or("unknown"),
                hit.content
            ));
        }
        Ok(context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// Spawn a one-shot stub KP server. Returns its base URL and a handle
    /// that resolves to the RAW request path the client actually sent —
    /// that's how the encoding tests assert on the wire bytes.
    async fn spawn_stub(body: String) -> (String, tokio::task::JoinHandle<String>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 16384];
            let n = sock.read(&mut buf).await.unwrap();
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
            let path = req.split_whitespace().nth(1).unwrap_or("").to_string();

            // Echo the received path inside response.query so tests can
            // assert on exactly what hit the wire.
            let payload = serde_json::json!({
                "total": 1,
                "mode": "semantic",
                "query": path,
                "results": [{
                    "chunk_id": "c1",
                    "document_id": "d1",
                    "content": "stub content",
                    "score": 0.9,
                    "title": "Stub Title",
                    "author": null,
                    "source": "stub.md",
                    "tags": ["stub"],
                    "highlights": [],
                    "provenance": {}
                }]
            });
            let response_json = if body.is_empty() {
                payload.to_string()
            } else {
                body
            };
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_json.len(),
                response_json
            );
            sock.write_all(resp.as_bytes()).await.unwrap();
            path
        });
        (format!("http://{addr}"), handle)
    }

    fn enabled_connector(endpoint: &str) -> KpConnector {
        KpConnector::new(KpConfig {
            enabled: true,
            endpoint: endpoint.to_string(),
        })
    }

    #[tokio::test]
    async fn query_is_percent_encoded_on_the_wire() {
        // Grill finding F11: the hand-rolled encoder broke on '%', quotes,
        // '&' and non-ASCII. reqwest .query() must encode all of them.
        let tricky = "brake \"controller\" & C++ %1 ± ą ł #tag";
        let (base, handle) = spawn_stub(String::new()).await;
        let hits = enabled_connector(&base)
            .semantic_search(tricky, 5)
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);

        let path = handle.await.unwrap();
        assert!(path.starts_with("/api/v1/search?q="), "path: {path}");
        // extract q=… up to the next param
        let q_value = path
            .trim_start_matches("/api/v1/search?q=")
            .split('&')
            .next()
            .unwrap();
        assert!(!q_value.contains(' '), "raw space leaked: {q_value}");
        assert!(!q_value.contains('&'), "raw ampersand leaked: {q_value}");
        assert!(!q_value.contains('"'), "raw quote leaked: {q_value}");
        assert!(
            !q_value.contains('%') || q_value.contains("%25"),
            "unencoded % leaked: {q_value}"
        );
        assert!(
            q_value.contains("%C4%85"),
            "ą not UTF-8 percent-encoded: {q_value}"
        );
    }

    #[tokio::test]
    async fn search_parses_verified_response_shape() {
        let (base, _handle) = spawn_stub(String::new()).await;
        let hits = enabled_connector(&base)
            .hybrid_search("anything", 3)
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].content, "stub content");
        assert_eq!(hits[0].title.as_deref(), Some("Stub Title"));
        assert_eq!(hits[0].source.as_deref(), Some("stub.md"));
        assert_eq!(hits[0].tags, vec!["stub".to_string()]);
        assert_eq!(hits[0].chunk_id, "c1");
        assert_eq!(hits[0].document_id, "d1");
        assert!((hits[0].score - 0.9).abs() < 1e-9);
        assert!(hits[0].author.is_none());
    }

    #[tokio::test]
    async fn health_check_requires_status_ok() {
        let (base, _handle) = spawn_stub(r#"{"status":"ok"}"#.to_string()).await;
        let ok = enabled_connector(&base).health_check().await.unwrap();
        assert!(ok);
    }

    #[tokio::test]
    async fn disabled_connector_fails_fast() {
        let connector = KpConnector::new(KpConfig::default());
        assert!(connector.semantic_search("q", 5).await.is_err());
        assert!(!connector.health_check().await.unwrap());
    }

    #[test]
    fn invalid_mode_rejected_before_http() {
        // No server needed — validation happens client-side.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let connector = KpConnector::new(KpConfig {
            enabled: true,
            endpoint: "http://127.0.0.1:1".to_string(),
        });
        let err = rt
            .block_on(connector.search_with_mode("q", "fuzzy", 5))
            .unwrap_err()
            .to_string();
        assert!(err.contains("invalid KP mode"), "got: {err}");
    }
}
