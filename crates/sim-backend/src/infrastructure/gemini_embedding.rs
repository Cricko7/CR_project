use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;

use crate::app::config::GeminiConfig;
use crate::infrastructure::gemini_rate_limiter::{GeminiRateLimiter, shared_gemini_rate_limiter};
use crate::memory::TextEmbedder;

const DEFAULT_EMBED_MODEL: &str = "text-embedding-004";

#[derive(Clone)]
pub struct GeminiEmbeddingClient {
    config: GeminiConfig,
    http: Client,
    embedding_model: String,
    rate_limiter: Arc<GeminiRateLimiter>,
}

impl GeminiEmbeddingClient {
    pub fn new(config: GeminiConfig) -> Result<Self> {
        let http = Client::builder()
            .timeout(config.timeout)
            .build()
            .context("failed to build HTTP client for Gemini embeddings")?;

        let embedding_model = if config.embedding_model.trim().is_empty() {
            DEFAULT_EMBED_MODEL.to_owned()
        } else {
            config.embedding_model.clone()
        };
        let rate_limiter = shared_gemini_rate_limiter(config.min_request_interval);

        Ok(Self {
            config,
            http,
            embedding_model,
            rate_limiter,
        })
    }

    fn endpoint(&self) -> String {
        let base = self.config.base_url.trim_end_matches('/');
        format!(
            "{base}/v1beta/models/{}:embedContent?key={}",
            self.embedding_model, self.config.api_key
        )
    }

    async fn embed_with_task(&self, text: &str, task_type: &str) -> Result<Vec<f32>> {
        let payload = json!({
            "content": {
                "parts": [{ "text": text }]
            },
            "taskType": task_type
        });

        self.rate_limiter.wait_turn().await;
        let response = self
            .http
            .post(self.endpoint())
            .json(&payload)
            .send()
            .await
            .context("failed to call Gemini embedding API")?;

        if !response.status().is_success() {
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "<no body>".to_owned());
            bail!("Gemini embedding request failed: {body}");
        }

        let parsed: GeminiEmbeddingResponse = response
            .json()
            .await
            .context("failed to parse Gemini embedding response")?;
        let values = parsed
            .embedding
            .and_then(|value| value.values)
            .ok_or_else(|| anyhow!("Gemini embedding response missing vector values"))?;
        Ok(values)
    }
}

#[async_trait]
impl TextEmbedder for GeminiEmbeddingClient {
    fn model_name(&self) -> &str {
        &self.embedding_model
    }

    async fn embed_document(&self, text: &str) -> Result<Vec<f32>> {
        self.embed_with_task(text, "RETRIEVAL_DOCUMENT").await
    }

    async fn embed_query(&self, text: &str) -> Result<Vec<f32>> {
        self.embed_with_task(text, "RETRIEVAL_QUERY").await
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiEmbeddingResponse {
    embedding: Option<GeminiEmbeddingValues>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiEmbeddingValues {
    values: Option<Vec<f32>>,
}
