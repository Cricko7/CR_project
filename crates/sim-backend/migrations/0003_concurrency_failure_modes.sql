CREATE TABLE IF NOT EXISTS agent_tick_dedup (
    agent_id UUID NOT NULL,
    tick_id TEXT NOT NULL,
    completed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (agent_id, tick_id)
);

CREATE TABLE IF NOT EXISTS agent_tick_locks (
    agent_id UUID PRIMARY KEY,
    tick_id TEXT NOT NULL,
    acquired_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_agent_tick_locks_expires_at
    ON agent_tick_locks(expires_at);

ALTER TABLE memory_entries
    ADD COLUMN IF NOT EXISTS embedding_claimed_at TIMESTAMPTZ;
