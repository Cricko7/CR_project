ALTER TABLE memory_entries
    ADD COLUMN IF NOT EXISTS is_summary BOOLEAN NOT NULL DEFAULT FALSE;

ALTER TABLE memory_entries
    ADD COLUMN IF NOT EXISTS archived BOOLEAN NOT NULL DEFAULT FALSE;

ALTER TABLE memory_entries
    ADD COLUMN IF NOT EXISTS summarized_by_id BIGINT REFERENCES memory_entries(id) ON DELETE SET NULL;

ALTER TABLE memory_entries
    ADD COLUMN IF NOT EXISTS embedding_model TEXT;

ALTER TABLE memory_entries
    ADD COLUMN IF NOT EXISTS embedding_error TEXT;

ALTER TABLE memory_entries
    ADD COLUMN IF NOT EXISTS last_accessed_at TIMESTAMPTZ;

CREATE INDEX IF NOT EXISTS idx_memory_entries_agent_active
    ON memory_entries(agent_id, archived, is_summary, created_at);

CREATE INDEX IF NOT EXISTS idx_memory_entries_pending_embeddings
    ON memory_entries(created_at)
    WHERE embedding_status = 'pending';
