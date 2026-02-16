ALTER TABLE memory_entries
    ADD COLUMN IF NOT EXISTS embedding_attempts INTEGER NOT NULL DEFAULT 0;

ALTER TABLE memory_entries
    ADD COLUMN IF NOT EXISTS embedding_next_retry_at TIMESTAMPTZ NOT NULL DEFAULT NOW();

ALTER TABLE memory_entries
    ADD COLUMN IF NOT EXISTS embedding_dead_lettered_at TIMESTAMPTZ;

UPDATE memory_entries
SET embedding_status = 'pending',
    embedding_next_retry_at = NOW()
WHERE embedding_status = 'failed';

CREATE INDEX IF NOT EXISTS idx_memory_entries_pending_retry_due
    ON memory_entries(embedding_next_retry_at, created_at)
    WHERE embedding_status = 'pending';

CREATE INDEX IF NOT EXISTS idx_memory_entries_dead_letter
    ON memory_entries(created_at DESC)
    WHERE embedding_status = 'dead_letter';
