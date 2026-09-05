CREATE TABLE IF NOT EXISTS item_groups (
    group_key   TEXT PRIMARY KEY NOT NULL
                CHECK (length(trim(group_key)) BETWEEN 1 AND 96),
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);

INSERT OR IGNORE INTO item_groups (group_key, created_at, updated_at)
SELECT group_key, MIN(created_at), MAX(updated_at)
FROM items
GROUP BY group_key;

INSERT OR IGNORE INTO item_groups (group_key, created_at, updated_at)
SELECT group_key, MIN(created_at), MAX(COALESCE(revoked_at, created_at))
FROM mcp_group_grants
GROUP BY group_key;

INSERT OR IGNORE INTO item_groups (group_key, created_at, updated_at)
VALUES (
    'inbox',
    CAST(strftime('%s', 'now') AS INTEGER) * 1000,
    CAST(strftime('%s', 'now') AS INTEGER) * 1000
);
