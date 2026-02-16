ALTER TABLE messages
    ADD COLUMN IF NOT EXISTS processing_claimed_at TIMESTAMPTZ;

ALTER TABLE messages
    ADD COLUMN IF NOT EXISTS delivery_attempts INTEGER NOT NULL DEFAULT 0;

ALTER TABLE messages
    ADD COLUMN IF NOT EXISTS delivery_error TEXT;

ALTER TABLE messages
    ADD COLUMN IF NOT EXISTS delivered_at TIMESTAMPTZ;

CREATE INDEX IF NOT EXISTS idx_messages_status_created_at
    ON messages(status, created_at);

CREATE INDEX IF NOT EXISTS idx_relationships_agent_a_affinity
    ON relationships(agent_a, affinity_score DESC);

CREATE INDEX IF NOT EXISTS idx_relationships_agent_b_affinity
    ON relationships(agent_b, affinity_score DESC);
