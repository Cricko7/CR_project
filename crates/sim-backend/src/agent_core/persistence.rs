use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct AgentRecord {
    pub id: Uuid,
    pub name: String,
    pub avatar_url: Option<String>,
    pub personality_json: Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct AgentStateRecord {
    pub agent_id: Uuid,
    pub valence: f32,
    pub arousal: f32,
    pub mood_label: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewAgentEvent {
    pub agent_id: Option<Uuid>,
    pub event_type: String,
    pub description: String,
    pub payload_json: Value,
}

#[derive(Debug, Clone)]
pub struct AgentEventRecord {
    pub id: i64,
    pub agent_id: Option<Uuid>,
    pub event_type: String,
    pub description: String,
    pub payload_json: Value,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct MessageRecord {
    pub id: i64,
    pub sender_type: String,
    pub sender_id: Option<Uuid>,
    pub receiver_agent_id: Uuid,
    pub content: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewMessage {
    pub sender_type: String,
    pub sender_id: Option<Uuid>,
    pub receiver_agent_id: Uuid,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct RelationshipRecord {
    pub id: i64,
    pub agent_a: Uuid,
    pub agent_b: Uuid,
    pub affinity_score: f32,
    pub history_summary: String,
    pub last_interaction_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TickLeaseAcquireResult {
    Acquired,
    Busy,
}

#[async_trait]
pub trait AgentCoreRepository: Send + Sync {
    async fn get_agent(&self, agent_id: Uuid) -> Result<Option<AgentRecord>>;
    async fn get_agent_state(&self, agent_id: Uuid) -> Result<Option<AgentStateRecord>>;
    async fn upsert_agent_state(&self, state: &AgentStateRecord) -> Result<()>;
    async fn append_agent_event(&self, event: &NewAgentEvent) -> Result<AgentEventRecord>;
    async fn list_agent_events(
        &self,
        agent_id: Option<Uuid>,
        limit: u32,
    ) -> Result<Vec<AgentEventRecord>>;
    async fn list_agent_events_after_id(
        &self,
        agent_id: Option<Uuid>,
        after_id: i64,
        limit: u32,
    ) -> Result<Vec<AgentEventRecord>>;
    async fn latest_event_id(&self, agent_id: Option<Uuid>) -> Result<Option<i64>>;
    async fn has_completed_tick(&self, agent_id: Uuid, tick_id: &str) -> Result<bool>;
    async fn try_acquire_tick_lease(
        &self,
        agent_id: Uuid,
        tick_id: &str,
        lease_ttl: Duration,
    ) -> Result<TickLeaseAcquireResult>;
    async fn release_tick_lease(&self, agent_id: Uuid, tick_id: &str) -> Result<()>;
    async fn record_completed_tick(&self, agent_id: Uuid, tick_id: &str) -> Result<()>;
    async fn enqueue_message(&self, new_message: &NewMessage) -> Result<MessageRecord>;
    async fn claim_queued_messages(
        &self,
        limit: u32,
        claim_timeout: Duration,
    ) -> Result<Vec<MessageRecord>>;
    async fn mark_message_delivered(&self, message_id: i64) -> Result<()>;
    async fn mark_message_failed(&self, message_id: i64, error: &str) -> Result<()>;
    async fn list_agent_messages(
        &self,
        receiver_agent_id: Uuid,
        limit: u32,
    ) -> Result<Vec<MessageRecord>>;
    async fn upsert_relationship_interaction(
        &self,
        left_agent_id: Uuid,
        right_agent_id: Uuid,
        affinity_delta: f32,
        interaction_summary: &str,
        interaction_at: DateTime<Utc>,
    ) -> Result<RelationshipRecord>;
    async fn list_agent_relationships(
        &self,
        agent_id: Uuid,
        limit: u32,
    ) -> Result<Vec<RelationshipRecord>>;
}
