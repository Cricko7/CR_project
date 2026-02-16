use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::Row;
use sqlx::postgres::PgPool;
use uuid::Uuid;

use crate::agent_core::{
    AgentCoreRepository, AgentEventRecord, AgentRecord, AgentStateRecord, NewAgentEvent,
};

#[derive(Clone)]
pub struct PostgresAgentCoreRepository {
    pool: PgPool,
}

impl PostgresAgentCoreRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl AgentCoreRepository for PostgresAgentCoreRepository {
    async fn get_agent(&self, agent_id: Uuid) -> Result<Option<AgentRecord>> {
        let row = sqlx::query(
            r#"
            SELECT id, name, avatar_url, personality_json, created_at
            FROM agents
            WHERE id = $1
            "#,
        )
        .bind(agent_id)
        .fetch_optional(&self.pool)
        .await
        .context("failed to fetch agent by id")?;

        Ok(row.map(map_agent_record))
    }

    async fn get_agent_state(&self, agent_id: Uuid) -> Result<Option<AgentStateRecord>> {
        let row = sqlx::query(
            r#"
            SELECT agent_id, valence, arousal, mood_label, updated_at
            FROM agent_states
            WHERE agent_id = $1
            "#,
        )
        .bind(agent_id)
        .fetch_optional(&self.pool)
        .await
        .context("failed to fetch agent state")?;

        Ok(row.map(map_agent_state_record))
    }

    async fn upsert_agent_state(&self, state: &AgentStateRecord) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO agent_states (agent_id, valence, arousal, mood_label, updated_at)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (agent_id)
            DO UPDATE SET
                valence = EXCLUDED.valence,
                arousal = EXCLUDED.arousal,
                mood_label = EXCLUDED.mood_label,
                updated_at = EXCLUDED.updated_at
            "#,
        )
        .bind(state.agent_id)
        .bind(state.valence)
        .bind(state.arousal)
        .bind(&state.mood_label)
        .bind(state.updated_at)
        .execute(&self.pool)
        .await
        .context("failed to upsert agent state")?;
        Ok(())
    }

    async fn append_agent_event(&self, event: &NewAgentEvent) -> Result<AgentEventRecord> {
        let row = sqlx::query(
            r#"
            INSERT INTO events (agent_id, event_type, description, payload_json)
            VALUES ($1, $2, $3, $4)
            RETURNING id, agent_id, event_type, description, payload_json, occurred_at
            "#,
        )
        .bind(event.agent_id)
        .bind(&event.event_type)
        .bind(&event.description)
        .bind(&event.payload_json)
        .fetch_one(&self.pool)
        .await
        .context("failed to append agent event")?;

        Ok(map_agent_event_record(row))
    }

    async fn list_agent_events(
        &self,
        agent_id: Option<Uuid>,
        limit: u32,
    ) -> Result<Vec<AgentEventRecord>> {
        let rows = if let Some(agent_id) = agent_id {
            sqlx::query(
                r#"
                SELECT id, agent_id, event_type, description, payload_json, occurred_at
                FROM events
                WHERE agent_id = $1
                ORDER BY occurred_at DESC
                LIMIT $2
                "#,
            )
            .bind(agent_id)
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await
            .context("failed to list events by agent id")?
        } else {
            sqlx::query(
                r#"
                SELECT id, agent_id, event_type, description, payload_json, occurred_at
                FROM events
                ORDER BY occurred_at DESC
                LIMIT $1
                "#,
            )
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await
            .context("failed to list events")?
        };

        Ok(rows.into_iter().map(map_agent_event_record).collect())
    }
}

fn map_agent_record(row: sqlx::postgres::PgRow) -> AgentRecord {
    AgentRecord {
        id: row.get::<Uuid, _>("id"),
        name: row.get::<String, _>("name"),
        avatar_url: row.get::<Option<String>, _>("avatar_url"),
        personality_json: row.get::<Value, _>("personality_json"),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
    }
}

fn map_agent_state_record(row: sqlx::postgres::PgRow) -> AgentStateRecord {
    AgentStateRecord {
        agent_id: row.get::<Uuid, _>("agent_id"),
        valence: row.get::<f32, _>("valence"),
        arousal: row.get::<f32, _>("arousal"),
        mood_label: row.get::<String, _>("mood_label"),
        updated_at: row.get::<DateTime<Utc>, _>("updated_at"),
    }
}

fn map_agent_event_record(row: sqlx::postgres::PgRow) -> AgentEventRecord {
    AgentEventRecord {
        id: row.get::<i64, _>("id"),
        agent_id: row.get::<Option<Uuid>, _>("agent_id"),
        event_type: row.get::<String, _>("event_type"),
        description: row.get::<String, _>("description"),
        payload_json: row.get::<Value, _>("payload_json"),
        occurred_at: row.get::<DateTime<Utc>, _>("occurred_at"),
    }
}
