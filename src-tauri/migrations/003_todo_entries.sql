CREATE TABLE IF NOT EXISTS todo_entries (
    id                  TEXT PRIMARY KEY NOT NULL,
    item_id             TEXT NOT NULL
                        REFERENCES items(id) ON DELETE CASCADE,
    text_ciphertext     BLOB NOT NULL,
    text_nonce          BLOB NOT NULL CHECK (length(text_nonce) = 24),
    text_version        INTEGER NOT NULL DEFAULT 1,
    is_completed        INTEGER NOT NULL DEFAULT 0
                        CHECK (is_completed IN (0, 1)),
    position            INTEGER NOT NULL CHECK (position >= 0),
    revision            INTEGER NOT NULL DEFAULT 1 CHECK (revision > 0),
    created_at          INTEGER NOT NULL,
    updated_at          INTEGER NOT NULL,
    completed_at        INTEGER
);

CREATE INDEX IF NOT EXISTS idx_todo_entries_item_position
    ON todo_entries(item_id, position, created_at);

CREATE INDEX IF NOT EXISTS idx_todo_entries_item_completed
    ON todo_entries(item_id, is_completed);
