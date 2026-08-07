PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS items (
    id                  TEXT PRIMARY KEY NOT NULL,
    kind                TEXT NOT NULL CHECK (kind IN ('note', 'todo', 'secret')),
    title               TEXT NOT NULL DEFAULT '',
    content_ciphertext  BLOB NOT NULL,
    content_nonce       BLOB NOT NULL CHECK (length(content_nonce) = 24),
    content_version     INTEGER NOT NULL DEFAULT 1,
    status              TEXT NOT NULL DEFAULT 'active'
                        CHECK (status IN ('active', 'completed', 'archived')),
    sensitivity         TEXT NOT NULL DEFAULT 'standard'
                        CHECK (sensitivity IN ('standard', 'sensitive')),
    color               TEXT NOT NULL DEFAULT 'sun'
                        CHECK (color IN ('sun', 'mint', 'sky', 'rose', 'lilac', 'paper')),
    tags_json           TEXT NOT NULL DEFAULT '[]',
    due_at              INTEGER,
    expires_at          INTEGER,
    completed_at        INTEGER,
    mcp_access          TEXT NOT NULL DEFAULT 'inherit'
                        CHECK (mcp_access IN ('inherit', 'deny', 'read', 'read_write')),
    revision            INTEGER NOT NULL DEFAULT 1 CHECK (revision > 0),
    created_at          INTEGER NOT NULL,
    updated_at          INTEGER NOT NULL,
    archived_at         INTEGER,
    CHECK (kind <> 'secret' OR (sensitivity = 'sensitive' AND mcp_access = 'deny'))
);

CREATE INDEX IF NOT EXISTS idx_items_status_updated
    ON items(status, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_items_kind_status
    ON items(kind, status);
CREATE INDEX IF NOT EXISTS idx_items_due_at
    ON items(due_at) WHERE due_at IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_items_expires_at
    ON items(expires_at) WHERE expires_at IS NOT NULL;

CREATE TABLE IF NOT EXISTS item_windows (
    item_id         TEXT PRIMARY KEY NOT NULL
                    REFERENCES items(id) ON DELETE CASCADE,
    x               REAL,
    y               REAL,
    width           REAL NOT NULL DEFAULT 340 CHECK (width >= 240),
    height          REAL NOT NULL DEFAULT 360 CHECK (height >= 180),
    monitor_name    TEXT,
    scale_factor    REAL NOT NULL DEFAULT 1 CHECK (scale_factor > 0),
    always_on_top   INTEGER NOT NULL DEFAULT 1 CHECK (always_on_top IN (0, 1)),
    opacity         REAL NOT NULL DEFAULT 1 CHECK (opacity >= 0.4 AND opacity <= 1),
    is_visible      INTEGER NOT NULL DEFAULT 0 CHECK (is_visible IN (0, 1)),
    updated_at      INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS settings (
    key         TEXT PRIMARY KEY NOT NULL,
    value_json  TEXT NOT NULL,
    updated_at  INTEGER NOT NULL
);

-- Reserved for the HTTP/stdio MCP layer. Tokens are represented only by hashes.
CREATE TABLE IF NOT EXISTS mcp_clients (
    id              TEXT PRIMARY KEY NOT NULL,
    display_name    TEXT NOT NULL,
    transport       TEXT NOT NULL CHECK (transport IN ('stdio', 'loopback_http', 'lan_http')),
    token_hash      BLOB,
    scopes_json     TEXT NOT NULL DEFAULT '[]',
    created_at      INTEGER NOT NULL,
    last_used_at    INTEGER,
    revoked_at      INTEGER
);

CREATE INDEX IF NOT EXISTS idx_mcp_clients_active
    ON mcp_clients(revoked_at) WHERE revoked_at IS NULL;
