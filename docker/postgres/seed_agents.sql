INSERT INTO agents (id, name, personality_json)
VALUES
    ('11111111-1111-1111-1111-111111111111', 'Alice', '{"traits":["curious","friendly"]}'::jsonb),
    ('22222222-2222-2222-2222-222222222222', 'Bob', '{"traits":["competitive","sarcastic"]}'::jsonb)
ON CONFLICT (id) DO UPDATE
SET
    name = EXCLUDED.name,
    personality_json = EXCLUDED.personality_json;
