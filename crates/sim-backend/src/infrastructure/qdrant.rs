use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use reqwest::header::{HeaderMap, HeaderValue};
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::app::config::QdrantConfig;
use crate::memory::{MemoryVectorStore, VectorSearchHit};

#[derive(Clone)]
pub struct QdrantVectorStore {
    config: QdrantConfig,
    http: Client,
}

impl QdrantVectorStore {
    pub fn new(config: QdrantConfig) -> Result<Self> {
        let mut headers = HeaderMap::new();
        if let Some(api_key) = &config.api_key {
            headers.insert(
                "api-key",
                HeaderValue::from_str(api_key).context("invalid Qdrant API key header value")?,
            );
        }

        let http = Client::builder()
            .default_headers(headers)
            .timeout(config.timeout)
            .build()
            .context("failed to build Qdrant HTTP client")?;

        Ok(Self { config, http })
    }

    fn base_url(&self) -> String {
        self.config.url.trim_end_matches('/').to_owned()
    }
}

#[async_trait]
impl MemoryVectorStore for QdrantVectorStore {
    async fn ensure_collection(&self) -> Result<()> {
        let url = format!("{}/collections/{}", self.base_url(), self.config.collection);
        let body = json!({
            "vectors": {
                "size": self.config.vector_size,
                "distance": "Cosine"
            }
        });

        let response = self
            .http
            .put(&url)
            .json(&body)
            .send()
            .await
            .context("failed to call Qdrant ensure_collection")?;

        if response.status().is_success() || response.status() == StatusCode::CONFLICT {
            return Ok(());
        }

        let error = response
            .text()
            .await
            .unwrap_or_else(|_| "<no body>".to_owned());
        bail!("qdrant ensure_collection failed: {error}");
    }

    async fn upsert_memory_vector(
        &self,
        memory_id: i64,
        agent_id: Uuid,
        vector: Vec<f32>,
        importance: f32,
        is_summary: bool,
        created_at: DateTime<Utc>,
    ) -> Result<()> {
        let url = format!(
            "{}/collections/{}/points?wait=true",
            self.base_url(),
            self.config.collection
        );
        let body = json!({
            "points": [{
                "id": memory_id,
                "vector": vector,
                "payload": {
                    "memory_id": memory_id,
                    "agent_id": agent_id.to_string(),
                    "importance": importance,
                    "is_summary": is_summary,
                    "created_at": created_at.to_rfc3339()
                }
            }]
        });

        let response = self
            .http
            .put(&url)
            .json(&body)
            .send()
            .await
            .context("failed to call Qdrant upsert")?;

        if response.status().is_success() {
            Ok(())
        } else {
            let error = response
                .text()
                .await
                .unwrap_or_else(|_| "<no body>".to_owned());
            bail!("qdrant upsert failed: {error}")
        }
    }

    async fn search_agent_memories(
        &self,
        agent_id: Uuid,
        query_vector: Vec<f32>,
        top_k: u32,
    ) -> Result<Vec<VectorSearchHit>> {
        let url = format!(
            "{}/collections/{}/points/search",
            self.base_url(),
            self.config.collection
        );
        let body = json!({
            "vector": query_vector,
            "limit": top_k,
            "with_payload": true,
            "filter": {
                "must": [{
                    "key": "agent_id",
                    "match": { "value": agent_id.to_string() }
                }]
            }
        });

        let response = self
            .http
            .post(&url)
            .json(&body)
            .send()
            .await
            .context("failed to call Qdrant search")?;

        if !response.status().is_success() {
            let error = response
                .text()
                .await
                .unwrap_or_else(|_| "<no body>".to_owned());
            bail!("qdrant search failed: {error}");
        }

        let parsed: QdrantSearchResponse = response
            .json()
            .await
            .context("failed to parse Qdrant search response")?;

        let mut hits = Vec::new();
        for point in parsed.result {
            let memory_id = point
                .payload
                .as_ref()
                .and_then(|payload| payload.memory_id)
                .or_else(|| parse_point_id(&point.id))
                .ok_or_else(|| anyhow!("qdrant hit missing memory_id"))?;
            hits.push(VectorSearchHit {
                memory_id,
                score: point.score as f32,
            });
        }
        Ok(hits)
    }
}

fn parse_point_id(id: &serde_json::Value) -> Option<i64> {
    if let Some(number) = id.as_i64() {
        return Some(number);
    }
    if let Some(text) = id.as_str() {
        return text.parse::<i64>().ok();
    }
    None
}

#[derive(Debug, Deserialize)]
struct QdrantSearchResponse {
    result: Vec<QdrantPoint>,
}

#[derive(Debug, Deserialize)]
struct QdrantPoint {
    id: serde_json::Value,
    score: f64,
    payload: Option<QdrantPayload>,
}

#[derive(Debug, Deserialize)]
struct QdrantPayload {
    memory_id: Option<i64>,
}
