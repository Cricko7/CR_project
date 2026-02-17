use std::time::Duration;

use anyhow::{Context, Result, ensure};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::Row;
use sqlx::postgres::PgPool;
use uuid::Uuid;

use crate::agent_core::{
    AgentCoreRepository, AgentEventRecord, AgentRecord, AgentStateRecord,
    DEFAULT_SIMULATION_TIME_SCALE, InterventionRecord, MessageRecord, NewAgent, NewAgentEvent,
    NewIntervention, NewMessage, RelationshipRecord, SimulationTimeScaleRecord,
    TickLeaseAcquireResult,
};

#[derive(Clone)]
pub struct PostgresAgentCoreRepository {
    pool: PgPool,
}

impl PostgresAgentCoreRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn apply_global_mood_decay(&self, step: f32) -> Result<u64> {
        let step = step.clamp(0.0001, 1.0);
        let result = sqlx::query(
            r#"
            WITH next_values AS (
                SELECT
                    agent_id,
                    CASE
                        WHEN ABS(valence) <= $1 THEN 0.0
                        ELSE valence - SIGN(valence) * $1
                    END AS next_valence,
                    CASE
                        WHEN ABS(arousal) <= $1 THEN 0.0
                        ELSE arousal - SIGN(arousal) * $1
                    END AS next_arousal
                FROM agent_states
            ),
            classified AS (
                SELECT
                    agent_id,
                    next_valence,
                    next_arousal,
                    CASE
                        WHEN next_valence >= 0.45 AND next_arousal >= 0.35 THEN 'excited'
                        WHEN next_valence >= 0.45 AND next_arousal <= -0.2 THEN 'content'
                        WHEN next_valence >= 0.2 AND next_arousal <= 0.3 THEN 'calm'
                        WHEN next_valence <= -0.45 AND next_arousal >= 0.35 THEN 'angry'
                        WHEN next_valence <= -0.35 AND next_arousal <= -0.15 THEN 'sad'
                        WHEN next_arousal >= 0.6 AND ABS(next_valence) < 0.2 THEN 'anxious'
                        WHEN next_arousal <= -0.45 AND ABS(next_valence) < 0.2 THEN 'tired'
                        ELSE 'neutral'
                    END AS next_mood_label
                FROM next_values
            )
            UPDATE agent_states AS state
            SET
                valence = classified.next_valence,
                arousal = classified.next_arousal,
                mood_label = classified.next_mood_label,
                updated_at = NOW()
            FROM classified
            WHERE state.agent_id = classified.agent_id
              AND (
                    state.valence IS DISTINCT FROM classified.next_valence
                 OR state.arousal IS DISTINCT FROM classified.next_arousal
                 OR state.mood_label IS DISTINCT FROM classified.next_mood_label
              )
            "#,
        )
        .bind(step)
        .execute(&self.pool)
        .await
        .context("failed to apply global mood decay")?;

        Ok(result.rows_affected())
    }
}

#[async_trait]
impl AgentCoreRepository for PostgresAgentCoreRepository {
    async fn create_agent(&self, new_agent: &NewAgent) -> Result<AgentRecord> {
        let mut tx = self
            .pool
            .begin()
            .await
            .context("failed to open transaction for agent creation")?;
        let agent_id = Uuid::new_v4();

        let row = sqlx::query(
            r#"
            INSERT INTO agents (id, name, avatar_url, personality_json)
            VALUES ($1, $2, $3, $4)
            RETURNING id, name, avatar_url, personality_json, created_at
            "#,
        )
        .bind(agent_id)
        .bind(&new_agent.name)
        .bind(&new_agent.avatar_url)
        .bind(&new_agent.personality_json)
        .fetch_one(&mut *tx)
        .await
        .context("failed to create agent")?;

        sqlx::query(
            r#"
            INSERT INTO agent_states (agent_id, valence, arousal, mood_label, updated_at)
            VALUES ($1, 0.0, 0.0, 'neutral', NOW())
            ON CONFLICT (agent_id) DO NOTHING
            "#,
        )
        .bind(agent_id)
        .execute(&mut *tx)
        .await
        .context("failed to initialize agent state")?;

        tx.commit()
            .await
            .context("failed to commit agent creation transaction")?;

        Ok(map_agent_record(row))
    }

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

    async fn list_agents(&self, limit: u32) -> Result<Vec<AgentRecord>> {
        let rows = sqlx::query(
            r#"
            SELECT id, name, avatar_url, personality_json, created_at
            FROM agents
            ORDER BY created_at ASC
            LIMIT $1
            "#,
        )
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .context("failed to list agents")?;

        Ok(rows.into_iter().map(map_agent_record).collect())
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

    async fn list_agent_events_after_id(
        &self,
        agent_id: Option<Uuid>,
        after_id: i64,
        limit: u32,
    ) -> Result<Vec<AgentEventRecord>> {
        let rows = if let Some(agent_id) = agent_id {
            sqlx::query(
                r#"
                SELECT id, agent_id, event_type, description, payload_json, occurred_at
                FROM events
                WHERE agent_id = $1
                  AND id > $2
                ORDER BY id ASC
                LIMIT $3
                "#,
            )
            .bind(agent_id)
            .bind(after_id)
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await
            .context("failed to list events after id by agent id")?
        } else {
            sqlx::query(
                r#"
                SELECT id, agent_id, event_type, description, payload_json, occurred_at
                FROM events
                WHERE id > $1
                ORDER BY id ASC
                LIMIT $2
                "#,
            )
            .bind(after_id)
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await
            .context("failed to list events after id")?
        };

        Ok(rows.into_iter().map(map_agent_event_record).collect())
    }

    async fn latest_event_id(&self, agent_id: Option<Uuid>) -> Result<Option<i64>> {
        let row = if let Some(agent_id) = agent_id {
            sqlx::query(
                r#"
                SELECT MAX(id) AS latest_id
                FROM events
                WHERE agent_id = $1
                "#,
            )
            .bind(agent_id)
            .fetch_one(&self.pool)
            .await
            .context("failed to fetch latest event id by agent id")?
        } else {
            sqlx::query(
                r#"
                SELECT MAX(id) AS latest_id
                FROM events
                "#,
            )
            .fetch_one(&self.pool)
            .await
            .context("failed to fetch latest event id")?
        };

        Ok(row.get::<Option<i64>, _>("latest_id"))
    }

    async fn has_completed_tick(&self, agent_id: Uuid, tick_id: &str) -> Result<bool> {
        let row = sqlx::query(
            r#"
            SELECT 1
            FROM agent_tick_dedup
            WHERE agent_id = $1 AND tick_id = $2
            "#,
        )
        .bind(agent_id)
        .bind(tick_id)
        .fetch_optional(&self.pool)
        .await
        .context("failed to check completed tick idempotency")?;

        Ok(row.is_some())
    }

    async fn try_acquire_tick_lease(
        &self,
        agent_id: Uuid,
        tick_id: &str,
        lease_ttl: Duration,
    ) -> Result<TickLeaseAcquireResult> {
        let ttl_ms = i64::try_from(lease_ttl.as_millis())
            .unwrap_or(i64::MAX)
            .max(1);
        let mut tx = self
            .pool
            .begin()
            .await
            .context("failed to open transaction for tick lease acquisition")?;

        sqlx::query(
            r#"
            DELETE FROM agent_tick_locks
            WHERE agent_id = $1
              AND expires_at <= NOW()
            "#,
        )
        .bind(agent_id)
        .execute(&mut *tx)
        .await
        .context("failed to clear expired tick lease")?;

        let insert_result = sqlx::query(
            r#"
            INSERT INTO agent_tick_locks (agent_id, tick_id, expires_at)
            VALUES ($1, $2, NOW() + ($3 * INTERVAL '1 millisecond'))
            ON CONFLICT (agent_id) DO NOTHING
            "#,
        )
        .bind(agent_id)
        .bind(tick_id)
        .bind(ttl_ms)
        .execute(&mut *tx)
        .await
        .context("failed to acquire tick lease")?;

        tx.commit()
            .await
            .context("failed to commit tick lease acquisition transaction")?;

        if insert_result.rows_affected() == 1 {
            Ok(TickLeaseAcquireResult::Acquired)
        } else {
            Ok(TickLeaseAcquireResult::Busy)
        }
    }

    async fn release_tick_lease(&self, agent_id: Uuid, tick_id: &str) -> Result<()> {
        sqlx::query(
            r#"
            DELETE FROM agent_tick_locks
            WHERE agent_id = $1
              AND tick_id = $2
            "#,
        )
        .bind(agent_id)
        .bind(tick_id)
        .execute(&self.pool)
        .await
        .context("failed to release tick lease")?;
        Ok(())
    }

    async fn record_completed_tick(&self, agent_id: Uuid, tick_id: &str) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO agent_tick_dedup (agent_id, tick_id)
            VALUES ($1, $2)
            ON CONFLICT (agent_id, tick_id) DO NOTHING
            "#,
        )
        .bind(agent_id)
        .bind(tick_id)
        .execute(&self.pool)
        .await
        .context("failed to persist completed tick idempotency record")?;
        Ok(())
    }

    async fn enqueue_message(&self, new_message: &NewMessage) -> Result<MessageRecord> {
        let row = sqlx::query(
            r#"
            INSERT INTO messages (sender_type, sender_id, receiver_agent_id, content, status)
            VALUES ($1, $2, $3, $4, 'queued')
            RETURNING id, sender_type, sender_id, receiver_agent_id, content, status, created_at
            "#,
        )
        .bind(&new_message.sender_type)
        .bind(new_message.sender_id)
        .bind(new_message.receiver_agent_id)
        .bind(&new_message.content)
        .fetch_one(&self.pool)
        .await
        .context("failed to enqueue message")?;

        Ok(map_message_record(row))
    }

    async fn claim_queued_messages(
        &self,
        limit: u32,
        claim_timeout: Duration,
    ) -> Result<Vec<MessageRecord>> {
        let timeout_ms = i64::try_from(claim_timeout.as_millis())
            .unwrap_or(i64::MAX)
            .max(1);
        let rows = sqlx::query(
            r#"
            WITH claimed AS (
                SELECT id
                FROM messages
                WHERE
                    status = 'queued'
                    OR (
                        status = 'processing'
                        AND processing_claimed_at IS NOT NULL
                        AND processing_claimed_at <= NOW() - ($2 * INTERVAL '1 millisecond')
                    )
                ORDER BY created_at ASC
                LIMIT $1
                FOR UPDATE SKIP LOCKED
            )
            UPDATE messages AS message
            SET status = 'processing',
                processing_claimed_at = NOW(),
                delivery_error = NULL
            FROM claimed
            WHERE message.id = claimed.id
            RETURNING message.id, message.sender_type, message.sender_id, message.receiver_agent_id,
                      message.content, message.status, message.created_at
            "#,
        )
        .bind(i64::from(limit))
        .bind(timeout_ms)
        .fetch_all(&self.pool)
        .await
        .context("failed to claim queued messages")?;

        Ok(rows.into_iter().map(map_message_record).collect())
    }

    async fn mark_message_delivered(&self, message_id: i64) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE messages
            SET status = 'delivered',
                processing_claimed_at = NULL,
                delivery_error = NULL,
                delivered_at = NOW()
            WHERE id = $1
            "#,
        )
        .bind(message_id)
        .execute(&self.pool)
        .await
        .context("failed to mark message delivered")?;
        Ok(())
    }

    async fn mark_message_failed(&self, message_id: i64, error: &str) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE messages
            SET status = 'failed',
                processing_claimed_at = NULL,
                delivery_attempts = delivery_attempts + 1,
                delivery_error = $2
            WHERE id = $1
            "#,
        )
        .bind(message_id)
        .bind(error)
        .execute(&self.pool)
        .await
        .context("failed to mark message failed")?;
        Ok(())
    }

    async fn list_agent_messages(
        &self,
        receiver_agent_id: Uuid,
        limit: u32,
    ) -> Result<Vec<MessageRecord>> {
        let rows = sqlx::query(
            r#"
            SELECT id, sender_type, sender_id, receiver_agent_id, content, status, created_at
            FROM messages
            WHERE receiver_agent_id = $1
            ORDER BY created_at DESC
            LIMIT $2
            "#,
        )
        .bind(receiver_agent_id)
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .context("failed to list agent messages")?;

        Ok(rows.into_iter().map(map_message_record).collect())
    }

    async fn list_agent_message_timeline(
        &self,
        agent_id: Uuid,
        limit: u32,
    ) -> Result<Vec<MessageRecord>> {
        let rows = sqlx::query(
            r#"
            SELECT id, sender_type, sender_id, receiver_agent_id, content, status, created_at
            FROM messages
            WHERE receiver_agent_id = $1
               OR sender_id = $1
            ORDER BY created_at DESC
            LIMIT $2
            "#,
        )
        .bind(agent_id)
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .context("failed to list agent message timeline")?;

        Ok(rows.into_iter().map(map_message_record).collect())
    }

    async fn upsert_relationship_interaction(
        &self,
        left_agent_id: Uuid,
        right_agent_id: Uuid,
        affinity_delta: f32,
        interaction_summary: &str,
        interaction_at: DateTime<Utc>,
    ) -> Result<RelationshipRecord> {
        let (agent_a, agent_b) = normalize_relationship_pair(left_agent_id, right_agent_id);
        let summary = interaction_summary.trim();
        let row = sqlx::query(
            r#"
            INSERT INTO relationships (
                agent_a,
                agent_b,
                affinity_score,
                history_summary,
                last_interaction_at
            )
            VALUES (
                $1,
                $2,
                LEAST(1.0, GREATEST(-1.0, $3)),
                LEFT($4, 2000),
                $5
            )
            ON CONFLICT (agent_a, agent_b)
            DO UPDATE SET
                affinity_score = LEAST(1.0, GREATEST(-1.0, relationships.affinity_score + $3)),
                history_summary = LEFT(
                    CASE
                        WHEN relationships.history_summary = '' THEN $4
                        WHEN $4 = '' THEN relationships.history_summary
                        ELSE relationships.history_summary || ' | ' || $4
                    END,
                    2000
                ),
                last_interaction_at = $5
            RETURNING id, agent_a, agent_b, affinity_score, history_summary, last_interaction_at, created_at
            "#,
        )
        .bind(agent_a)
        .bind(agent_b)
        .bind(affinity_delta.clamp(-1.0, 1.0))
        .bind(summary)
        .bind(interaction_at)
        .fetch_one(&self.pool)
        .await
        .context("failed to upsert relationship interaction")?;

        Ok(map_relationship_record(row))
    }

    async fn list_agent_relationships(
        &self,
        agent_id: Uuid,
        limit: u32,
    ) -> Result<Vec<RelationshipRecord>> {
        let rows = sqlx::query(
            r#"
            SELECT id, agent_a, agent_b, affinity_score, history_summary, last_interaction_at, created_at
            FROM relationships
            WHERE agent_a = $1 OR agent_b = $1
            ORDER BY COALESCE(last_interaction_at, created_at) DESC
            LIMIT $2
            "#,
        )
        .bind(agent_id)
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .context("failed to list agent relationships")?;

        Ok(rows.into_iter().map(map_relationship_record).collect())
    }

    async fn list_relationships(&self, limit: u32) -> Result<Vec<RelationshipRecord>> {
        let rows = sqlx::query(
            r#"
            SELECT id, agent_a, agent_b, affinity_score, history_summary, last_interaction_at, created_at
            FROM relationships
            ORDER BY COALESCE(last_interaction_at, created_at) DESC
            LIMIT $1
            "#,
        )
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .context("failed to list relationships")?;

        Ok(rows.into_iter().map(map_relationship_record).collect())
    }

    async fn append_intervention(
        &self,
        intervention: &NewIntervention,
    ) -> Result<InterventionRecord> {
        let row = sqlx::query(
            r#"
            INSERT INTO interventions (admin_user_id, action_type, payload_json, result_status)
            VALUES ($1, $2, $3, $4)
            RETURNING id, admin_user_id, action_type, payload_json, result_status, created_at
            "#,
        )
        .bind(&intervention.admin_user_id)
        .bind(&intervention.action_type)
        .bind(&intervention.payload_json)
        .bind(&intervention.result_status)
        .fetch_one(&self.pool)
        .await
        .context("failed to append intervention")?;

        Ok(map_intervention_record(row))
    }

    async fn list_interventions(&self, limit: u32) -> Result<Vec<InterventionRecord>> {
        let rows = sqlx::query(
            r#"
            SELECT id, admin_user_id, action_type, payload_json, result_status, created_at
            FROM interventions
            ORDER BY created_at DESC
            LIMIT $1
            "#,
        )
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .context("failed to list interventions")?;

        Ok(rows.into_iter().map(map_intervention_record).collect())
    }

    async fn get_time_scale(&self) -> Result<SimulationTimeScaleRecord> {
        sqlx::query(
            r#"
            INSERT INTO simulation_controls (id, time_scale)
            VALUES (TRUE, $1)
            ON CONFLICT (id) DO NOTHING
            "#,
        )
        .bind(DEFAULT_SIMULATION_TIME_SCALE)
        .execute(&self.pool)
        .await
        .context("failed to initialize simulation controls singleton row")?;

        let row = sqlx::query(
            r#"
            SELECT time_scale, updated_at
            FROM simulation_controls
            WHERE id = TRUE
            "#,
        )
        .fetch_one(&self.pool)
        .await
        .context("failed to read simulation time scale")?;

        Ok(map_simulation_time_scale_record(row))
    }

    async fn set_time_scale(&self, time_scale: f32) -> Result<SimulationTimeScaleRecord> {
        ensure!(
            time_scale.is_finite() && time_scale > 0.0,
            "time_scale must be a positive finite value"
        );

        let row = sqlx::query(
            r#"
            INSERT INTO simulation_controls (id, time_scale)
            VALUES (TRUE, $1)
            ON CONFLICT (id)
            DO UPDATE SET
                time_scale = EXCLUDED.time_scale,
                updated_at = NOW()
            RETURNING time_scale, updated_at
            "#,
        )
        .bind(time_scale)
        .fetch_one(&self.pool)
        .await
        .context("failed to update simulation time scale")?;

        Ok(map_simulation_time_scale_record(row))
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

fn map_message_record(row: sqlx::postgres::PgRow) -> MessageRecord {
    MessageRecord {
        id: row.get::<i64, _>("id"),
        sender_type: row.get::<String, _>("sender_type"),
        sender_id: row.get::<Option<Uuid>, _>("sender_id"),
        receiver_agent_id: row.get::<Uuid, _>("receiver_agent_id"),
        content: row.get::<String, _>("content"),
        status: row.get::<String, _>("status"),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
    }
}

fn map_relationship_record(row: sqlx::postgres::PgRow) -> RelationshipRecord {
    RelationshipRecord {
        id: row.get::<i64, _>("id"),
        agent_a: row.get::<Uuid, _>("agent_a"),
        agent_b: row.get::<Uuid, _>("agent_b"),
        affinity_score: row.get::<f32, _>("affinity_score"),
        history_summary: row.get::<String, _>("history_summary"),
        last_interaction_at: row.get::<Option<DateTime<Utc>>, _>("last_interaction_at"),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
    }
}

fn map_intervention_record(row: sqlx::postgres::PgRow) -> InterventionRecord {
    InterventionRecord {
        id: row.get::<i64, _>("id"),
        admin_user_id: row.get::<String, _>("admin_user_id"),
        action_type: row.get::<String, _>("action_type"),
        payload_json: row.get::<Value, _>("payload_json"),
        result_status: row.get::<String, _>("result_status"),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
    }
}

fn map_simulation_time_scale_record(row: sqlx::postgres::PgRow) -> SimulationTimeScaleRecord {
    SimulationTimeScaleRecord {
        time_scale: row.get::<f32, _>("time_scale"),
        updated_at: row.get::<DateTime<Utc>, _>("updated_at"),
    }
}

fn normalize_relationship_pair(left_agent_id: Uuid, right_agent_id: Uuid) -> (Uuid, Uuid) {
    if left_agent_id.as_u128() <= right_agent_id.as_u128() {
        (left_agent_id, right_agent_id)
    } else {
        (right_agent_id, left_agent_id)
    }
}
