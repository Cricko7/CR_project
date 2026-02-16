use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::Row;
use sqlx::postgres::PgPool;
use uuid::Uuid;

use crate::memory::{MemoryEntryRecord, MemoryRepository, NewMemoryEntry};

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

    async fn list_pending_embeddings(&self, limit: u32) -> Result<Vec<MemoryEntryRecord>> {
        let rows = sqlx::query(
            r#"
            SELECT id, agent_id, content, summary, importance, is_summary, archived, embedding_status, created_at
            FROM memory_entries
            WHERE embedding_status = 'pending'
            ORDER BY created_at ASC
            LIMIT $1
            "#,
        )
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .context("failed to list pending memory embeddings")?;

        Ok(rows.into_iter().map(map_memory_entry).collect())
    }

    async fn mark_embedding_done(&self, memory_id: i64, embedding_model: &str) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE memory_entries
            SET embedding_status = 'embedded', embedding_model = $2, embedding_error = NULL
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

    async fn mark_embedding_failed(&self, memory_id: i64, error: &str) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE memory_entries
            SET embedding_status = 'failed', embedding_error = $2
            WHERE id = $1
            "#,
        )
        .bind(memory_id)
        .bind(error)
        .execute(&self.pool)
        .await
        .context("failed to mark memory embedding as failed")?;
        Ok(())
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
