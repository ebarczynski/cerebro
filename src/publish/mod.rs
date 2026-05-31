//! Confluence publishing pipeline.
//!
//! Converts markdown notes to Confluence storage format and publishes
//! via the Atlassian Confluence REST API v2.

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfluenceConfig {
    pub enabled: bool,
    pub base_url: String,
    pub space_key: String,
    pub default_parent_page: Option<String>,
    pub publish_tag: String,
}

impl Default for ConfluenceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            base_url: String::new(),
            space_key: String::new(),
            default_parent_page: None,
            publish_tag: "confluence".to_string(),
        }
    }
}

#[derive(Debug, Serialize)]
struct ConfluencePageCreate {
    space_key: String,
    status: String,
    title: String,
    #[serde(rename = "body")]
    body: ConfluenceBody,
}

#[derive(Debug, Serialize)]
struct ConfluenceBody {
    storage: ConfluenceStorage,
    representation: String,
}

#[derive(Debug, Serialize)]
struct ConfluenceStorage {
    value: String,
    representation: String,
}

pub struct ConfluencePublisher {
    client: reqwest::Client,
    config: ConfluenceConfig,
    auth_token: Option<String>,
}

impl ConfluencePublisher {
    pub fn new(config: ConfluenceConfig) -> Self {
        Self {
            client: reqwest::Client::new(),
            config,
            auth_token: None,
        }
    }

    pub fn with_auth(mut self, token: String) -> Self {
        self.auth_token = Some(token);
        self
    }

    /// Convert markdown to Confluence storage format (simplified).
    pub fn markdown_to_confluence(&self, markdown: &str) -> String {
        let mut html = String::from("<ac:structured-macro ac:name=\"info\"><ac:rich-text-body>");
        html.push_str("<p><em>Published from Cerebro Knowledge Base</em></p>");
        html.push_str("</ac:rich-text-body></ac:structured-macro>\n\n");

        for line in markdown.lines() {
            if line.starts_with("# ") {
                html.push_str(&format!("<h1>{}</h1>\n", &line[2..]));
            } else if line.starts_with("## ") {
                html.push_str(&format!("<h2>{}</h2>\n", &line[3..]));
            } else if line.starts_with("### ") {
                html.push_str(&format!("<h3>{}</h3>\n", &line[4..]));
            } else if line.starts_with("- ") || line.starts_with("* ") {
                html.push_str(&format!("<li>{}</li>\n", &line[2..]));
            } else if line.starts_with("```") {
                html.push_str("<ac:structured-macro ac:name=\"code\"><ac:plain-text-body><![CDATA[");
                // Will be closed on next ```
            } else if line.starts_with("---") {
                // Skip frontmatter delimiters
            } else if line.contains("[[") && line.contains("]]") {
                // Convert wikilinks to plain text for Confluence
                let cleaned = regex_replace_wikilinks(line);
                html.push_str(&format!("<p>{}</p>\n", cleaned));
            } else if !line.is_empty() {
                html.push_str(&format!("<p>{}</p>\n", line));
            }
        }

        html
    }

    /// Publish a note to Confluence (dry-run mode returns the HTML).
    pub async fn publish(
        &self,
        title: &str,
        content: &str,
        parent_page_id: Option<&str>,
        dry_run: bool,
    ) -> Result<String> {
        if !self.config.enabled {
            return Err(anyhow!("Confluence publishing is disabled"));
        }

        let html = self.markdown_to_confluence(content);

        if dry_run {
            return Ok(format!("DRY RUN — Would publish to Confluence:\n\nSpace: {}\nTitle: {}\nParent: {}\n\nHTML preview (first 500 chars):\n{}...",
                self.config.space_key,
                title,
                parent_page_id.unwrap_or("root"),
                &html[..html.len().min(500)]
            ));
        }

        let token = self.auth_token.as_ref()
            .ok_or_else(|| anyhow!("Confluence auth token not configured"))?;

        let url = format!("{}/rest/api/content/", self.config.base_url);

        let page = ConfluencePageCreate {
            space_key: self.config.space_key.clone(),
            status: "current".to_string(),
            title: title.to_string(),
            body: ConfluenceBody {
                storage: ConfluenceStorage {
                    value: html,
                    representation: "storage".to_string(),
                },
                representation: "storage".to_string(),
            },
        };

        let resp = self.client
            .post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .header("Content-Type", "application/json")
            .json(&page)
            .send()
            .await?;

        if resp.status().is_success() {
            let result: Value = resp.json().await?;
            let page_id = result.get("id").and_then(|v| v.as_str()).unwrap_or("unknown");
            Ok(format!("Published to Confluence: {} (ID: {})", title, page_id))
        } else {
            let status = resp.status();
            let body = resp.text().await?;
            Err(anyhow!("Confluence API error {}: {}", status, body))
        }
    }
}

use serde_json::Value;

fn regex_replace_wikilinks(line: &str) -> String {
    let re = regex::Regex::new(r"\[\[([^\]|]+)(?:\|([^\]]+))?\]\]").unwrap();
    re.replace_all(line, |caps: &regex::Captures| {
        let alias = caps.get(2).map(|m| m.as_str()).unwrap_or_else(|| caps.get(1).unwrap().as_str());
        alias.to_string()
    }).to_string()
}
