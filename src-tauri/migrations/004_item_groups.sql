ALTER TABLE items
    ADD COLUMN group_key TEXT NOT NULL DEFAULT 'inbox'
    CHECK (length(trim(group_key)) BETWEEN 1 AND 96);

CREATE INDEX IF NOT EXISTS idx_items_group_status_updated
    ON items(group_key, status, updated_at DESC);
