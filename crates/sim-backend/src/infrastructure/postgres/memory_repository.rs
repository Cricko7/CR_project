use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::Row;
use sqlx::postgres::PgPool;
use uuid::Uuid;

use crate::memory::{
    EmbeddingFailureDisposition, MemoryEntryRecord, MemoryRepository, NewMemoryEntry,
};

const EMBEDDING_MAX_ATTEMPTS: i32 = 5;
const EMBEDDING_RETRY_BACKOFF_MS: i64 = 5_000;

#[derive(Clone)]
pub struct PostgresMemoryRepository {
    pool: PgPool,
}

impl PostgresMemoryRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl MemoryRepository for PostgresMemoryRepository {
    async fn insert_memory_entry(&self, new_entry: &NewMemoryEntry) -> Result<MemoryEntryRecord> {
        let row = sqlx::query(
            r#"
            INSERT INTO memory_entries (agent_id, content, summary, importance, is_summary, embedding_status)
            VALUES ($1, $2, $3, $4, $5, 'pending')
            RETURNING id, agent_id, content, summary, importance, is_summary, archived, embedding_status, created_at
            "#,
        )
        .bind(new_entry.agent_id)
        .bind(&new_entry.content)
        .bind(&new_entry.summary)
        .bind(new_entry.importance)
        .bind(new_entry.is_summary)
        .fetch_one(&self.pool)
        .await
        .context("failed to insert memory entry")?;

        Ok(map_memory_entry(row))
    }

    async fn claim_pending_embeddings(
        &self,
        limit: u32,
        claim_timeout: Duration,
    ) -> Result<Vec<MemoryEntryRecord>> {
        let timeout_ms = i64::try_from(claim_timeout.as_millis())
            .unwrap_or(i64::MAX)
            .max(1);
        let rows = sqlx::query(
            r#"
            WITH claimed AS (
                SELECT id
                FROM memory_entries
                WHERE
                    (
                        embedding_status = 'pending'
                        AND embedding_next_retry_at <= NOW()
                    )
                    OR (
                        embedding_status = 'processing'
                        AND embedding_claimed_at IS NOT NULL
                        AND embedding_claimed_at <= NOW() - ($2 * INTERVAL '1 millisecond')
                    )
                ORDER BY created_at ASC
                LIMIT $1
                FOR UPDATE SKIP LOCKED
            )
            UPDATE memory_entries AS memory
            SET embedding_status = 'processing',
                embedding_claimed_at = NOW(),
                embedding_error = NULL
            FROM claimed
            WHERE memory.id = claimed.id
            RETURNING memory.id, memory.agent_id, memory.content, memory.summary, memory.importance,
                      memory.is_summary, memory.archived, memory.embedding_status, memory.created_at
            "#,
        )
        .bind(i64::from(limit))
        .bind(timeout_ms)
        .fetch_all(&self.pool)
        .await
        .context("failed to claim pending memory embeddings")?;

        Ok(rows.into_iter().map(map_memory_entry).collect())
    }

    async fn mark_embedding_done(&self, memory_id: i64, embedding_model: &str) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE memory_entries
            SET embedding_status = 'embedded',
                embedding_model = $2,
                embedding_error = NULL,
                embedding_claimed_at = NULL,
                embedding_next_retry_at = NOW()
            WHERE id = $1
            "#,
        )
        .bind(memory_id)
        .bind(embedding_model)
        .execute(&self.pool)
        .await
        .context("failed to mark memory embedding as done")?;
        Ok(())
    }

    async fn mark_embedding_failed(
        &self,
        memory_id: i64,
        error: &str,
    ) -> Result<EmbeddingFailureDisposition> {
        let row = sqlx::query(
            r#"
            UPDATE memory_entries
            SET embedding_status = CASE
                    WHEN embedding_attempts + 1 >= $3 THEN 'dead_letter'
                    ELSE 'pending'
                END,
                embedding_error = $2,
                embedding_claimed_at = NULL,
                embedding_attempts = embedding_attempts + 1,
                embedding_next_retry_at = CASE
                    WHEN embedding_attempts + 1 >= $3 THEN NOW()
                    ELSE NOW() + ($4 * INTERVAL '1 millisecond')
                END,
                embedding_dead_lettered_at = CASE
                    WHEN embedding_attempts + 1 >= $3 THEN NOW()
                    ELSE NULL
                END
            WHERE id = $1
            RETURNING embedding_status
            "#,
        )
        .bind(memory_id)
        .bind(error)
        .bind(EMBEDDING_MAX_ATTEMPTS)
        .bind(EMBEDDING_RETRY_BACKOFF_MS)
        .fetch_optional(&self.pool)
        .await
        .context("failed to mark memory embedding as failed")?;

        let status = row
            .as_ref()
            .map(|value| value.get::<String, _>("embedding_status"))
            .ok_or_else(|| anyhow!("memory entry `{memory_id}` was not found"))?;
        if status == "dead_letter" {
            Ok(EmbeddingFailureDisposition::DeadLettered)
        } else {
            Ok(EmbeddingFailureDisposition::RetryScheduled)
        }
    }

    async fn list_dead_letter_embeddings(&self, limit: u32) -> Result<Vec<MemoryEntryRecord>> {
        let rows = sqlx::query(
            r#"
            SELECT id, agent_id, content, summary, importance, is_summary, archived, embedding_status, created_at
            FROM memory_entries
            WHERE embedding_status = 'dead_letter'
            ORDER BY created_at DESC
            LIMIT $1
            "#,
        )
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .context("failed to list dead-letter memory embeddings")?;

        Ok(rows.into_iter().map(map_memory_entry).collect())
    }

    async fn requeue_dead_letter_embedding(&self, memory_id: i64) -> Result<bool> {
        let result = sqlx::query(
            r#"
            UPDATE memory_entries
            SET embedding_status = 'pending',
                embedding_error = NULL,
                embedding_attempts = 0,
                embedding_claimed_at = NULL,
                embedding_next_retry_at = NOW(),
                embedding_dead_lettered_at = NULL
            WHERE id = $1
              AND embedding_status = 'dead_letter'
            "#,
        )
        .bind(memory_id)
        .execute(&self.pool)
        .await
        .context("failed to requeue dead-letter memory embedding")?;

        Ok(result.rows_affected() == 1)
    }

    async fn list_memories_by_ids(&self, ids: &[i64]) -> Result<Vec<MemoryEntryRecord>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }

        let rows = sqlx::query(
            r#"
            SELECT id, agent_id, content, summary, importance, is_summary, archived, embedding_status, created_at
            FROM memory_entries
            WHERE id = ANY($1)
            "#,
        )
        .bind(ids)
        .fetch_all(&self.pool)
        .await
        .context("failed to list memories by ids")?;

        Ok(rows.into_iter().map(map_memory_entry).collect())
    }

    async fn list_oldest_active_memories(
        &self,
        agent_id: Uuid,
        limit: u32,
    ) -> Result<Vec<MemoryEntryRecord>> {
        let rows = sqlx::query(
            r#"
            SELECT id, agent_id, content, summary, importance, is_summary, archived, embedding_status, created_at
            FROM memory_entries
            WHERE agent_id = $1
              AND archived = FALSE
              AND is_summary = FALSE
            ORDER BY created_at ASC
            LIMIT $2
            "#,
        )
        .bind(agent_id)
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .context("failed to list oldest active memories")?;

        Ok(rows.into_iter().map(map_memory_entry).collect())
    }

    async fn count_active_memories(&self, agent_id: Uuid) -> Result<u64> {
        let row = sqlx::query(
            r#"
            SELECT COUNT(*)::BIGINT AS total
            FROM memory_entries
            WHERE agent_id = $1
              AND archived = FALSE
              AND is_summary = FALSE
            "#,
        )
        .bind(agent_id)
        .fetch_one(&self.pool)
        .await
        .context("failed to count active memories")?;

        let total: i64 = row.get("total");
        Ok(total.max(0) as u64)
    }

    async fn list_recent_memories(
        &self,
        agent_id: Uuid,
        limit: u32,
    ) -> Result<Vec<MemoryEntryRecord>> {
        let rows = sqlx::query(
            r#"
            SELECT id, agent_id, content, summary, importance, is_summary, archived, embedding_status, created_at
            FROM memory_entries
            WHERE agent_id = $1
              AND archived = FALSE
            ORDER BY created_at DESC
            LIMIT $2
            "#,
        )
        .bind(agent_id)
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .context("failed to list recent memories")?;

        Ok(rows.into_iter().map(map_memory_entry).collect())
    }

    async fn archive_memories(&self, ids: &[i64], summarized_by_id: i64) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }

        sqlx::query(
            r#"
            UPDATE memory_entries
            SET archived = TRUE, summarized_by_id = $2
            WHERE id = ANY($1)
            "#,
        )
        .bind(ids)
        .bind(summarized_by_id)
        .execute(&self.pool)
        .await
        .context("failed to archive memories")?;
        Ok(())
    }
}

fn map_memory_entry(row: sqlx::postgres::PgRow) -> MemoryEntryRecord {
    MemoryEntryRecord {
        id: row.get::<i64, _>("id"),
        agent_id: row.get::<Uuid, _>("agent_id"),
        content: row.get::<String, _>("content"),
        summary: row.get::<Option<String>, _>("summary"),
        importance: row.get::<f32, _>("importance"),
        is_summary: row.get::<bool, _>("is_summary"),
        archived: row.get::<bool, _>("archived"),
        embedding_status: row.get::<String, _>("embedding_status"),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
    }
}
