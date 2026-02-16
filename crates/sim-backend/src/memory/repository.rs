use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddingFailureDisposition {
    RetryScheduled,
    DeadLettered,
}

#[derive(Debug, Clone)]
pub struct MemoryEntryRecord {
    pub id: i64,
    pub agent_id: Uuid,
    pub content: String,
    pub summary: Option<String>,
    pub importance: f32,
    pub is_summary: bool,
    pub archived: bool,
    pub embedding_status: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewMemoryEntry {
    pub agent_id: Uuid,
    pub content: String,
    pub summary: Option<String>,
    pub importance: f32,
    pub is_summary: bool,
}

#[async_trait]
pub trait MemoryRepository: Send + Sync {
    async fn insert_memory_entry(&self, new_entry: &NewMemoryEntry) -> Result<MemoryEntryRecord>;
    async fn claim_pending_embeddings(
        &self,
        limit: u32,
        claim_timeout: Duration,
    ) -> Result<Vec<MemoryEntryRecord>>;
    async fn mark_embedding_done(&self, memory_id: i64, embedding_model: &str) -> Result<()>;
    async fn mark_embedding_failed(
        &self,
        memory_id: i64,
        error: &str,
    ) -> Result<EmbeddingFailureDisposition>;
    async fn list_dead_letter_embeddings(&self, limit: u32) -> Result<Vec<MemoryEntryRecord>>;
    async fn requeue_dead_letter_embedding(&self, memory_id: i64) -> Result<bool>;
    async fn list_memories_by_ids(&self, ids: &[i64]) -> Result<Vec<MemoryEntryRecord>>;
    async fn list_oldest_active_memories(
        &self,
        agent_id: Uuid,
        limit: u32,
    ) -> Result<Vec<MemoryEntryRecord>>;
    async fn list_recent_memories(
        &self,
        agent_id: Uuid,
        limit: u32,
    ) -> Result<Vec<MemoryEntryRecord>>;
    async fn count_active_memories(&self, agent_id: Uuid) -> Result<u64>;
    async fn archive_memories(&self, ids: &[i64], summarized_by_id: i64) -> Result<()>;
}
