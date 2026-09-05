CREATE TABLE IF NOT EXISTS mcp_group_grants (
    client_id      TEXT NOT NULL
                   REFERENCES mcp_clients(id) ON DELETE CASCADE,
    group_key      TEXT NOT NULL
                   CHECK (length(trim(group_key)) BETWEEN 1 AND 96),
    role           TEXT NOT NULL
                   CHECK (role IN ('viewer', 'contributor', 'coordinator')),
    created_at     INTEGER NOT NULL,
    revoked_at     INTEGER,
    PRIMARY KEY (client_id, group_key)
);

CREATE INDEX IF NOT EXISTS idx_mcp_group_grants_active_group
    ON mcp_group_grants(group_key, role, client_id)
    WHERE revoked_at IS NULL;

CREATE TABLE IF NOT EXISTS task_events (
    event_id        INTEGER PRIMARY KEY AUTOINCREMENT,
    item_id         TEXT NOT NULL
                    REFERENCES items(id) ON DELETE CASCADE,
    group_key       TEXT NOT NULL,
    actor_client_id TEXT
                    REFERENCES mcp_clients(id) ON DELETE SET NULL,
    action          TEXT NOT NULL,
    item_revision   INTEGER NOT NULL CHECK (item_revision > 0),
    payload_json    TEXT NOT NULL DEFAULT '{}',
    created_at      INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_task_events_group_cursor
    ON task_events(group_key, event_id);

INSERT OR IGNORE INTO settings (key, value_json, updated_at)
VALUES (
    'mcp.server',
    '{"enabled":true,"bindAddress":"127.0.0.1","port":4784,"allowLan":false}',
    CAST(strftime('%s', 'now') AS INTEGER) * 1000
);
