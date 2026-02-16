use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct VectorSearchHit {
    pub memory_id: i64,
    pub score: f32,
}

#[async_trait]
pub trait MemoryVectorStore: Send + Sync {
    async fn ensure_collection(&self) -> Result<()>;

    async fn upsert_memory_vector(
        &self,
        memory_id: i64,
        agent_id: Uuid,
        vector: Vec<f32>,
        importance: f32,
        is_summary: bool,
        created_at: DateTime<Utc>,
    ) -> Result<()>;

    async fn search_agent_memories(
        &self,
        agent_id: Uuid,
        query_vector: Vec<f32>,
        top_k: u32,
    ) -> Result<Vec<VectorSearchHit>>;
}
