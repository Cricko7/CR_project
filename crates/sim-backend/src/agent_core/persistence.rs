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

#[async_trait]
pub trait AgentCoreRepository: Send + Sync {
    async fn get_agent(&self, agent_id: Uuid) -> Result<Option<AgentRecord>>;
    async fn get_agent_state(&self, agent_id: Uuid) -> Result<Option<AgentStateRecord>>;
    async fn upsert_agent_state(&self, state: &AgentStateRecord) -> Result<()>;
    async fn append_agent_event(&self, event: &NewAgentEvent) -> Result<AgentEventRecord>;
}
