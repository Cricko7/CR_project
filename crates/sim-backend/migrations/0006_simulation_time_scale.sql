CREATE TABLE IF NOT EXISTS simulation_controls (
    id BOOLEAN PRIMARY KEY DEFAULT TRUE,
    time_scale REAL NOT NULL DEFAULT 1.0,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT chk_simulation_controls_singleton CHECK (id = TRUE),
    CONSTRAINT chk_simulation_time_scale_positive CHECK (time_scale > 0.0)
);

INSERT INTO simulation_controls (id, time_scale)
VALUES (TRUE, 1.0)
ON CONFLICT (id) DO NOTHING;
