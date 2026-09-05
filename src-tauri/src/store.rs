use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::{Mutex, MutexGuard},
    time::{SystemTime, UNIX_EPOCH},
};

use chacha20poly1305::{
    Key, XChaCha20Poly1305, XNonce,
    aead::{Aead, Generate, KeyInit, Payload},
};
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};
use sha2::{Digest, Sha256};

use crate::mcp_model::{
    CreateMcpClientInput, CreatedMcpClient, McpClientView, McpCreateGroupInput, McpCreateItemInput,
    McpGroupGrantView, McpGroupRole, McpItemView, McpListItemsInput, McpMoveItemInput,
    McpPrincipal, McpRenameGroupInput, McpServerConfig, McpSetStatusInput,
    McpSetTodoEntryCompletedInput, McpTodoEntryDraftInput, McpTodoEntryView, McpTransport,
    McpUpdateItemInput, SetMcpGroupGrantInput,
};
use crate::model::{
    AddTodoEntryInput, CreateGroupInput, CreateItemInput, DeleteTodoEntryInput, Item, ItemColor,
    ItemGroup, ItemKind, ItemStatus, LocalePreference, McpAccess, RenameGroupInput, Sensitivity,
    SetStatusInput, SetTodoEntryCompletedInput, ThemeMode, TodoEntry, TodoEntryDraftInput,
    UpdateItemInput, UpdateTodoEntryInput, WindowGeometryInput, WindowOpacityInput, WindowState,
};

const INITIAL_MIGRATION: &str = include_str!("../migrations/001_initial.sql");
const WINDOW_OPACITY_MIGRATION: &str = include_str!("../migrations/002_window_opacity.sql");
const TODO_ENTRIES_MIGRATION: &str = include_str!("../migrations/003_todo_entries.sql");
const ITEM_GROUPS_MIGRATION: &str = include_str!("../migrations/004_item_groups.sql");
const MCP_V1_MIGRATION: &str = include_str!("../migrations/005_mcp_v1.sql");
const PERSISTENT_GROUPS_MIGRATION: &str = include_str!("../migrations/006_persistent_groups.sql");
const LATEST_SCHEMA_VERSION: i64 = 6;
const MAX_TITLE_CHARS: usize = 160;
const MAX_GROUP_CHARS: usize = 96;
const MAX_CONTENT_BYTES: usize = 1024 * 1024;
const MAX_TODO_ENTRIES: usize = 200;
const MAX_TODO_ENTRY_CHARS: usize = 500;
const MAX_TIMESTAMP_MS: i64 = 253_402_300_799_999;
const DEFAULT_SECRET_TTL_MS: i64 = 24 * 60 * 60 * 1000;
const LOCALE_SETTING_KEY: &str = "appearance.locale";
const THEME_SETTING_KEY: &str = "appearance.theme";
const MCP_SERVER_SETTING_KEY: &str = "mcp.server";
const MCP_GROUP_MANAGEMENT_SCOPE: &str = "groups:manage";
const MAX_MCP_LIST_ITEMS: u32 = 200;

const ITEM_SELECT: &str = r#"
SELECT
    i.id,
    i.kind,
    i.group_key,
    i.title,
    i.content_ciphertext,
    i.content_nonce,
    i.status,
    i.sensitivity,
    i.color,
    i.tags_json,
    i.due_at,
    i.expires_at,
    i.completed_at,
    i.mcp_access,
    i.revision,
    i.created_at,
    i.updated_at,
    i.archived_at,
    w.x,
    w.y,
    w.width,
    w.height,
    w.monitor_name,
    w.scale_factor,
    w.always_on_top,
    w.background_opacity,
    w.text_opacity,
    w.is_visible,
    w.updated_at
FROM items i
LEFT JOIN item_windows w ON w.item_id = i.id
"#;

const MCP_ITEM_SELECT: &str = r#"
SELECT
    i.id,
    i.kind,
    i.group_key,
    i.title,
    i.content_ciphertext,
    i.content_nonce,
    i.status,
    i.tags_json,
    i.due_at,
    i.completed_at,
    i.revision,
    i.created_at,
    i.updated_at,
    i.archived_at
FROM items i
"#;

pub struct Store {
    connection: Mutex<Connection>,
    key: [u8; 32],
}

struct StoredItem {
    id: String,
    kind: String,
    group_key: String,
    title: String,
    ciphertext: Vec<u8>,
    nonce: Vec<u8>,
    status: String,
    sensitivity: String,
    color: String,
    tags_json: String,
    due_at: Option<i64>,
    expires_at: Option<i64>,
    completed_at: Option<i64>,
    mcp_access: String,
    revision: i64,
    created_at: i64,
    updated_at: i64,
    archived_at: Option<i64>,
    window_x: Option<f64>,
    window_y: Option<f64>,
    window_width: Option<f64>,
    window_height: Option<f64>,
    monitor_name: Option<String>,
    scale_factor: Option<f64>,
    always_on_top: Option<i64>,
    background_opacity: Option<f64>,
    text_opacity: Option<f64>,
    is_visible: Option<i64>,
    window_updated_at: Option<i64>,
}

struct StoredTodoEntry {
    id: String,
    item_id: String,
    ciphertext: Vec<u8>,
    nonce: Vec<u8>,
    is_completed: bool,
    position: i64,
    revision: i64,
    created_at: i64,
    updated_at: i64,
    completed_at: Option<i64>,
}

struct StoredMcpItem {
    id: String,
    kind: String,
    group_key: String,
    title: String,
    ciphertext: Vec<u8>,
    nonce: Vec<u8>,
    status: String,
    tags_json: String,
    due_at: Option<i64>,
    completed_at: Option<i64>,
    revision: i64,
    created_at: i64,
    updated_at: i64,
    archived_at: Option<i64>,
}

fn migrate(connection: &mut Connection) -> Result<(), String> {
    let mut version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|error| format!("read database version failed: {error}"))?;

    if version > LATEST_SCHEMA_VERSION {
        return Err(format!(
            "database version {version} is newer than supported version {LATEST_SCHEMA_VERSION}"
        ));
    }

    if version == 0 {
        let transaction = connection
            .transaction()
            .map_err(|error| format!("start initial migration failed: {error}"))?;
        transaction
            .execute_batch(INITIAL_MIGRATION)
            .map_err(|error| format!("initial database migration failed: {error}"))?;
        transaction
            .pragma_update(None, "user_version", 1_i64)
            .map_err(|error| format!("record initial database version failed: {error}"))?;
        transaction
            .commit()
            .map_err(|error| format!("commit initial database migration failed: {error}"))?;
        version = 1;
    }

    if version == 1 {
        let transaction = connection
            .transaction()
            .map_err(|error| format!("start opacity migration failed: {error}"))?;
        transaction
            .execute_batch(WINDOW_OPACITY_MIGRATION)
            .map_err(|error| format!("window opacity migration failed: {error}"))?;
        transaction
            .pragma_update(None, "user_version", 2_i64)
            .map_err(|error| format!("record opacity migration version failed: {error}"))?;
        transaction
            .commit()
            .map_err(|error| format!("commit opacity migration failed: {error}"))?;
        version = 2;
    }

    if version == 2 {
        let transaction = connection
            .transaction()
            .map_err(|error| format!("start TODO entries migration failed: {error}"))?;
        transaction
            .execute_batch(TODO_ENTRIES_MIGRATION)
            .map_err(|error| format!("TODO entries migration failed: {error}"))?;
        transaction
            .pragma_update(None, "user_version", 3_i64)
            .map_err(|error| format!("record TODO entries migration version failed: {error}"))?;
        transaction
            .commit()
            .map_err(|error| format!("commit TODO entries migration failed: {error}"))?;
        version = 3;
    }

    if version == 3 {
        let transaction = connection
            .transaction()
            .map_err(|error| format!("start item groups migration failed: {error}"))?;
        transaction
            .execute_batch(ITEM_GROUPS_MIGRATION)
            .map_err(|error| format!("item groups migration failed: {error}"))?;
        transaction
            .pragma_update(None, "user_version", 4_i64)
            .map_err(|error| format!("record item groups migration version failed: {error}"))?;
        transaction
            .commit()
            .map_err(|error| format!("commit item groups migration failed: {error}"))?;
        version = 4;
    }

    if version == 4 {
        let transaction = connection
            .transaction()
            .map_err(|error| format!("start MCP v1 migration failed: {error}"))?;
        transaction
            .execute_batch(MCP_V1_MIGRATION)
            .map_err(|error| format!("MCP v1 migration failed: {error}"))?;
        transaction
            .pragma_update(None, "user_version", 5_i64)
            .map_err(|error| format!("record MCP v1 migration version failed: {error}"))?;
        transaction
            .commit()
            .map_err(|error| format!("commit MCP v1 migration failed: {error}"))?;
        version = 5;
    }

    if version == 5 {
        let transaction = connection
            .transaction()
            .map_err(|error| format!("start persistent groups migration failed: {error}"))?;
        transaction
            .execute_batch(PERSISTENT_GROUPS_MIGRATION)
            .map_err(|error| format!("persistent groups migration failed: {error}"))?;
        transaction
            .pragma_update(None, "user_version", 6_i64)
            .map_err(|error| {
                format!("record persistent groups migration version failed: {error}")
            })?;
        transaction
            .commit()
            .map_err(|error| format!("commit persistent groups migration failed: {error}"))?;
    }

    Ok(())
}

impl Store {
    pub fn open(path: &Path, key: [u8; 32]) -> Result<Self, String> {
        let connection =
            Connection::open(path).map_err(|error| format!("open database failed: {error}"))?;
        Self::from_connection(connection, key)
    }

    fn from_connection(mut connection: Connection, key: [u8; 32]) -> Result<Self, String> {
        connection
            .busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|error| format!("configure database timeout failed: {error}"))?;
        connection
            .execute_batch(
                "PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL; PRAGMA foreign_keys = ON;",
            )
            .map_err(|error| format!("configure database failed: {error}"))?;
        migrate(&mut connection)?;

        Ok(Self {
            connection: Mutex::new(connection),
            key,
        })
    }

    pub fn create_mcp_client(
        &self,
        input: CreateMcpClientInput,
    ) -> Result<CreatedMcpClient, String> {
        let display_name = input.display_name.trim();
        if display_name.is_empty() {
            return Err("MCP client display name cannot be empty".to_string());
        }
        if display_name.chars().count() > 80 {
            return Err("MCP client display name cannot exceed 80 characters".to_string());
        }

        let id = uuid::Uuid::new_v4().to_string();
        let bearer_token = format!(
            "tips_{id}_{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let token_hash = hash_mcp_token(&bearer_token);
        let now = now_ms();
        self.lock()?
            .execute(
                r#"
                INSERT INTO mcp_clients (
                    id, display_name, transport, token_hash, scopes_json,
                    created_at, last_used_at, revoked_at
                ) VALUES (?1, ?2, ?3, ?4, '[]', ?5, NULL, NULL)
                "#,
                params![id, display_name, input.transport.as_str(), token_hash, now],
            )
            .map_err(|error| format!("create MCP client failed: {error}"))?;

        Ok(CreatedMcpClient {
            client: McpClientView {
                id,
                display_name: display_name.to_string(),
                transport: input.transport,
                can_manage_groups: false,
                created_at: now,
                last_used_at: None,
                revoked_at: None,
            },
            bearer_token,
        })
    }

    pub fn authenticate_mcp_client(
        &self,
        bearer_token: &str,
        transport: McpTransport,
    ) -> Result<McpPrincipal, String> {
        let client_id = parse_mcp_client_id(bearer_token)
            .ok_or_else(|| "invalid MCP client credentials".to_string())?;
        let candidate_hash = hash_mcp_token(bearer_token);
        let connection = self.lock()?;
        let stored = connection
            .query_row(
                r#"
                SELECT display_name, transport, token_hash
                FROM mcp_clients
                WHERE id = ?1 AND revoked_at IS NULL AND token_hash IS NOT NULL
                "#,
                [&client_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| format!("authenticate MCP client failed: {error}"))?
            .ok_or_else(|| "invalid MCP client credentials".to_string())?;

        if !constant_time_eq(&stored.2, &candidate_hash) || stored.1 != transport.as_str() {
            return Err("invalid MCP client credentials".to_string());
        }

        connection
            .execute(
                "UPDATE mcp_clients SET last_used_at = ?1 WHERE id = ?2 AND revoked_at IS NULL",
                params![now_ms(), client_id],
            )
            .map_err(|error| format!("record MCP client use failed: {error}"))?;

        Ok(McpPrincipal {
            client_id,
            display_name: stored.0,
            transport,
        })
    }

    pub fn list_mcp_clients(&self, include_revoked: bool) -> Result<Vec<McpClientView>, String> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                r#"
                SELECT id, display_name, transport, scopes_json, created_at, last_used_at, revoked_at
                FROM mcp_clients
                WHERE ?1 = 1 OR revoked_at IS NULL
                ORDER BY revoked_at IS NOT NULL, display_name COLLATE NOCASE, id
                "#,
            )
            .map_err(|error| format!("prepare MCP client list failed: {error}"))?;
        let rows = statement
            .query_map([bool_to_int(include_revoked)], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                    row.get::<_, Option<i64>>(6)?,
                ))
            })
            .map_err(|error| format!("query MCP client list failed: {error}"))?;
        rows.map(|row| {
            let (id, display_name, transport, scopes_json, created_at, last_used_at, revoked_at) =
                row.map_err(|error| format!("read MCP client failed: {error}"))?;
            Ok(McpClientView {
                id,
                display_name,
                transport: McpTransport::from_db(&transport)?,
                can_manage_groups: mcp_scope_enabled(&scopes_json, MCP_GROUP_MANAGEMENT_SCOPE)?,
                created_at,
                last_used_at,
                revoked_at,
            })
        })
        .collect()
    }

    pub fn set_mcp_group_management(&self, client_id: &str, allowed: bool) -> Result<(), String> {
        let connection = self.lock()?;
        let scopes_json = connection
            .query_row(
                "SELECT scopes_json FROM mcp_clients WHERE id = ?1 AND revoked_at IS NULL",
                [client_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| format!("read MCP client capabilities failed: {error}"))?
            .ok_or_else(|| "active MCP client not found".to_string())?;
        let mut scopes = parse_mcp_scopes(&scopes_json)?;
        if allowed {
            if !scopes
                .iter()
                .any(|scope| scope == MCP_GROUP_MANAGEMENT_SCOPE)
            {
                scopes.push(MCP_GROUP_MANAGEMENT_SCOPE.to_string());
            }
        } else {
            scopes.retain(|scope| scope != MCP_GROUP_MANAGEMENT_SCOPE);
        }
        scopes.sort();
        scopes.dedup();
        let scopes_json = serde_json::to_string(&scopes)
            .map_err(|error| format!("encode MCP client capabilities failed: {error}"))?;
        connection
            .execute(
                "UPDATE mcp_clients SET scopes_json = ?1 WHERE id = ?2 AND revoked_at IS NULL",
                params![scopes_json, client_id],
            )
            .map_err(|error| format!("update MCP client capabilities failed: {error}"))?;
        Ok(())
    }

    pub fn revoke_mcp_client(&self, client_id: &str) -> Result<(), String> {
        let changed = self
            .lock()?
            .execute(
                "UPDATE mcp_clients SET revoked_at = ?1 WHERE id = ?2 AND revoked_at IS NULL",
                params![now_ms(), client_id],
            )
            .map_err(|error| format!("revoke MCP client failed: {error}"))?;
        if changed == 0 {
            return Err("active MCP client not found".to_string());
        }
        Ok(())
    }

    pub fn list_groups(&self) -> Result<Vec<ItemGroup>, String> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                r#"
                SELECT
                    g.group_key,
                    COUNT(i.id),
                    COALESCE(SUM(CASE WHEN i.status <> 'archived' THEN 1 ELSE 0 END), 0),
                    g.created_at,
                    g.updated_at
                FROM item_groups g
                LEFT JOIN items i ON i.group_key = g.group_key
                GROUP BY g.group_key, g.created_at, g.updated_at
                ORDER BY CASE WHEN g.group_key = 'inbox' THEN 0 ELSE 1 END,
                         g.group_key COLLATE NOCASE
                "#,
            )
            .map_err(|error| format!("prepare group list failed: {error}"))?;
        let rows = statement
            .query_map([], |row| {
                Ok(ItemGroup {
                    key: row.get(0)?,
                    item_count: row.get(1)?,
                    visible_item_count: row.get(2)?,
                    created_at: row.get(3)?,
                    updated_at: row.get(4)?,
                })
            })
            .map_err(|error| format!("query groups failed: {error}"))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| format!("read group failed: {error}"))
    }

    pub fn create_group(&self, input: CreateGroupInput) -> Result<ItemGroup, String> {
        let group = normalize_group(Some(&input.group))?;
        let now = now_ms();
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction()
            .map_err(|error| format!("start group creation failed: {error}"))?;
        Self::insert_group(&transaction, &group, now)?;
        let result = Self::get_group_with_connection(&transaction, &group)?;
        transaction
            .commit()
            .map_err(|error| format!("commit group creation failed: {error}"))?;
        Ok(result)
    }

    pub fn rename_group(&self, input: RenameGroupInput) -> Result<ItemGroup, String> {
        let group = normalize_group(Some(&input.group))?;
        let new_group = normalize_group(Some(&input.new_group))?;
        let now = now_ms();
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction()
            .map_err(|error| format!("start group rename failed: {error}"))?;
        Self::rename_group_in_transaction(&transaction, &group, &new_group, now)?;
        let result = Self::get_group_with_connection(&transaction, &new_group)?;
        transaction
            .commit()
            .map_err(|error| format!("commit group rename failed: {error}"))?;
        Ok(result)
    }

    pub fn mcp_create_group(
        &self,
        client_id: &str,
        input: McpCreateGroupInput,
    ) -> Result<ItemGroup, String> {
        let group = normalize_group(Some(&input.group))?;
        let now = now_ms();
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction()
            .map_err(|error| format!("start MCP group creation failed: {error}"))?;
        Self::assert_mcp_group_management(&transaction, client_id)?;
        Self::insert_group(&transaction, &group, now)?;
        transaction
            .execute(
                r#"
                INSERT INTO mcp_group_grants (client_id, group_key, role, created_at, revoked_at)
                VALUES (?1, ?2, 'coordinator', ?3, NULL)
                ON CONFLICT(client_id, group_key) DO UPDATE SET
                    role = 'coordinator',
                    created_at = excluded.created_at,
                    revoked_at = NULL
                "#,
                params![client_id, group, now],
            )
            .map_err(|error| {
                format!("grant MCP coordinator access to new group failed: {error}")
            })?;
        let result = Self::get_group_with_connection(&transaction, &group)?;
        transaction
            .commit()
            .map_err(|error| format!("commit MCP group creation failed: {error}"))?;
        Ok(result)
    }

    pub fn mcp_rename_group(
        &self,
        client_id: &str,
        input: McpRenameGroupInput,
    ) -> Result<ItemGroup, String> {
        let group = normalize_group(Some(&input.group))?;
        let new_group = normalize_group(Some(&input.new_group))?;
        let now = now_ms();
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction()
            .map_err(|error| format!("start MCP group rename failed: {error}"))?;
        Self::assert_mcp_group_management(&transaction, client_id)?;
        Self::assert_mcp_group_coordinator(&transaction, client_id, &group)?;
        Self::rename_group_in_transaction(&transaction, &group, &new_group, now)?;
        let result = Self::get_group_with_connection(&transaction, &new_group)?;
        transaction
            .commit()
            .map_err(|error| format!("commit MCP group rename failed: {error}"))?;
        Ok(result)
    }

    pub fn set_mcp_group_grant(
        &self,
        input: SetMcpGroupGrantInput,
    ) -> Result<McpGroupGrantView, String> {
        let group = normalize_group(Some(&input.group))?;
        let now = now_ms();
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction()
            .map_err(|error| format!("start MCP group grant update failed: {error}"))?;
        let client_exists: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM mcp_clients WHERE id = ?1 AND revoked_at IS NULL)",
                [&input.client_id],
                |row| row.get(0),
            )
            .map_err(|error| format!("validate MCP client failed: {error}"))?;
        if !client_exists {
            return Err("active MCP client not found".to_string());
        }
        Self::ensure_group(&transaction, &group, now)?;
        transaction
            .execute(
                r#"
                INSERT INTO mcp_group_grants (client_id, group_key, role, created_at, revoked_at)
                VALUES (?1, ?2, ?3, ?4, NULL)
                ON CONFLICT(client_id, group_key) DO UPDATE SET
                    role = excluded.role,
                    created_at = excluded.created_at,
                    revoked_at = NULL
                "#,
                params![input.client_id, group, input.role.as_str(), now],
            )
            .map_err(|error| format!("save MCP group grant failed: {error}"))?;
        transaction
            .commit()
            .map_err(|error| format!("commit MCP group grant failed: {error}"))?;
        Ok(McpGroupGrantView {
            client_id: input.client_id,
            group,
            role: input.role,
            created_at: now,
            revoked_at: None,
        })
    }

    pub fn list_mcp_group_grants(
        &self,
        client_id: Option<&str>,
        include_revoked: bool,
    ) -> Result<Vec<McpGroupGrantView>, String> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                r#"
                SELECT client_id, group_key, role, created_at, revoked_at
                FROM mcp_group_grants
                WHERE (?1 IS NULL OR client_id = ?1)
                  AND (?2 = 1 OR revoked_at IS NULL)
                ORDER BY group_key, client_id
                "#,
            )
            .map_err(|error| format!("prepare MCP group grant list failed: {error}"))?;
        let rows = statement
            .query_map(params![client_id, bool_to_int(include_revoked)], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                ))
            })
            .map_err(|error| format!("query MCP group grants failed: {error}"))?;
        rows.map(|row| {
            let (client_id, group, role, created_at, revoked_at) =
                row.map_err(|error| format!("read MCP group grant failed: {error}"))?;
            Ok(McpGroupGrantView {
                client_id,
                group,
                role: McpGroupRole::from_db(&role)?,
                created_at,
                revoked_at,
            })
        })
        .collect()
    }

    pub fn mcp_list_groups(&self, client_id: &str) -> Result<Vec<String>, String> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                r#"
                SELECT g.group_key
                FROM mcp_group_grants g
                JOIN item_groups ig ON ig.group_key = g.group_key
                JOIN mcp_clients c ON c.id = g.client_id
                WHERE g.client_id = ?1
                  AND g.revoked_at IS NULL
                  AND c.revoked_at IS NULL
                ORDER BY g.group_key
                "#,
            )
            .map_err(|error| format!("prepare MCP group list failed: {error}"))?;
        let rows = statement
            .query_map([client_id], |row| row.get::<_, String>(0))
            .map_err(|error| format!("query MCP group list failed: {error}"))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| format!("read MCP group failed: {error}"))
    }

    pub fn revoke_mcp_group_grant(&self, client_id: &str, group: &str) -> Result<(), String> {
        let group = normalize_group(Some(group))?;
        let changed = self
            .lock()?
            .execute(
                r#"
                UPDATE mcp_group_grants
                SET revoked_at = ?1
                WHERE client_id = ?2 AND group_key = ?3 AND revoked_at IS NULL
                "#,
                params![now_ms(), client_id, group],
            )
            .map_err(|error| format!("revoke MCP group grant failed: {error}"))?;
        if changed == 0 {
            return Err("active MCP group grant not found".to_string());
        }
        Ok(())
    }

    pub fn get_mcp_server_config(&self) -> Result<McpServerConfig, String> {
        let value = self
            .lock()?
            .query_row(
                "SELECT value_json FROM settings WHERE key = ?1",
                [MCP_SERVER_SETTING_KEY],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| format!("read MCP server setting failed: {error}"))?;
        match value {
            Some(value) => serde_json::from_str(&value)
                .map_err(|error| format!("decode MCP server setting failed: {error}")),
            None => Ok(McpServerConfig::default()),
        }
    }

    pub fn mcp_list_items(
        &self,
        client_id: &str,
        input: McpListItemsInput,
    ) -> Result<Vec<McpItemView>, String> {
        let group = normalize_group(Some(&input.group))?;
        let kind = input.kind.map(|value| value.as_str().to_string());
        let status = input.status.map(|value| value.as_str().to_string());
        let limit = i64::from(input.limit.unwrap_or(100).clamp(1, MAX_MCP_LIST_ITEMS));
        let sql = format!(
            r#"
            {MCP_ITEM_SELECT}
            WHERE i.group_key = ?1
              AND (?2 IS NULL OR i.kind = ?2)
              AND (?3 IS NULL OR i.status = ?3)
              AND (?3 IS NOT NULL OR i.status <> 'archived')
              AND (?4 IS NULL OR i.updated_at > ?4)
              AND i.kind IN ('note', 'todo')
              AND i.sensitivity = 'standard'
              AND i.mcp_access <> 'deny'
              AND EXISTS (
                  SELECT 1
                  FROM mcp_group_grants g
                  JOIN mcp_clients c ON c.id = g.client_id
                  WHERE g.client_id = ?5
                    AND g.group_key = i.group_key
                    AND g.revoked_at IS NULL
                    AND c.revoked_at IS NULL
              )
            ORDER BY i.updated_at DESC, i.id
            LIMIT ?6
            "#
        );
        let connection = self.lock()?;
        let stored_items = {
            let mut statement = connection
                .prepare(&sql)
                .map_err(|error| format!("prepare MCP item list failed: {error}"))?;
            let rows = statement
                .query_map(
                    params![group, kind, status, input.updated_after, client_id, limit,],
                    Self::read_stored_mcp_item,
                )
                .map_err(|error| format!("query MCP item list failed: {error}"))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|error| format!("read MCP item failed: {error}"))?
        };

        stored_items
            .into_iter()
            .map(|stored| self.decrypt_mcp_item(&connection, stored))
            .collect()
    }

    pub fn mcp_get_item(&self, client_id: &str, item_id: &str) -> Result<McpItemView, String> {
        let connection = self.lock()?;
        self.get_mcp_item_with_connection(&connection, client_id, item_id)
    }

    pub fn mcp_create_item(
        &self,
        client_id: &str,
        input: McpCreateItemInput,
    ) -> Result<McpItemView, String> {
        if input.kind == ItemKind::Secret {
            return Err("MCP cannot create secret items".to_string());
        }
        validate_optional_timestamp_ms("dueAt", input.due_at)?;
        let group = normalize_group(Some(&input.group))?;
        let todo_entries = normalize_initial_todo_entries(input.kind, input.todo_entries)?;
        validate_item_content(&input.title, &input.content, !todo_entries.is_empty())?;
        let title = normalize_title(&input.title);
        let tags = normalize_tags(input.tags);
        let tags_json = serde_json::to_string(&tags)
            .map_err(|error| format!("serialize tags failed: {error}"))?;
        let now = now_ms();
        let id = uuid::Uuid::new_v4().to_string();
        let (ciphertext, nonce) = self.encrypt_content(&id, &input.content)?;

        let item = {
            let mut connection = self.lock()?;
            let transaction = connection
                .transaction()
                .map_err(|error| format!("start MCP item creation failed: {error}"))?;
            Self::assert_mcp_group_write(&transaction, client_id, &group)?;
            transaction
                .execute(
                    r#"
                    INSERT INTO items (
                        id, kind, group_key, title, content_ciphertext, content_nonce,
                        content_version, status, sensitivity, color, tags_json, due_at,
                        expires_at, completed_at, mcp_access, revision, created_at,
                        updated_at, archived_at
                    ) VALUES (
                        ?1, ?2, ?3, ?4, ?5, ?6,
                        1, 'active', 'standard', 'sun', ?7, ?8,
                        NULL, NULL, 'read_write', 1, ?9,
                        ?9, NULL
                    )
                    "#,
                    params![
                        id,
                        input.kind.as_str(),
                        group,
                        title,
                        ciphertext,
                        nonce,
                        tags_json,
                        input.due_at,
                        now,
                    ],
                )
                .map_err(|error| format!("insert MCP item failed: {error}"))?;
            transaction
                .execute(
                    r#"
                    INSERT INTO item_windows (
                        item_id, width, height, scale_factor, always_on_top,
                        opacity, is_visible, updated_at
                    ) VALUES (?1, 340, 360, 1, 1, 1, 0, ?2)
                    "#,
                    params![id, now],
                )
                .map_err(|error| format!("insert window state failed: {error}"))?;

            for (position, text) in todo_entries.iter().enumerate() {
                let entry_id = uuid::Uuid::new_v4().to_string();
                let (ciphertext, nonce) = self.encrypt_todo_entry(&id, &entry_id, text)?;
                transaction
                    .execute(
                        r#"
                        INSERT INTO todo_entries (
                            id, item_id, text_ciphertext, text_nonce, text_version,
                            is_completed, position, revision, created_at, updated_at, completed_at
                        ) VALUES (?1, ?2, ?3, ?4, 1, 0, ?5, 1, ?6, ?6, NULL)
                        "#,
                        params![entry_id, id, ciphertext, nonce, position as i64, now],
                    )
                    .map_err(|error| format!("insert MCP TODO entry failed: {error}"))?;
            }
            Self::record_mcp_event(&transaction, &id, &group, client_id, "item.created", 1, now)?;
            let item = self.get_mcp_item_with_connection(&transaction, client_id, &id)?;
            transaction
                .commit()
                .map_err(|error| format!("commit MCP item creation failed: {error}"))?;
            item
        };

        Ok(item)
    }

    pub fn mcp_update_item(
        &self,
        client_id: &str,
        input: McpUpdateItemInput,
    ) -> Result<McpItemView, String> {
        validate_optional_timestamp_ms("dueAt", input.due_at)?;
        let todo_entries = input
            .todo_entries
            .map(|entries| {
                entries
                    .into_iter()
                    .map(|entry: McpTodoEntryDraftInput| TodoEntryDraftInput {
                        id: entry.id,
                        expected_revision: entry.expected_revision,
                        text: entry.text,
                        is_completed: entry.is_completed,
                    })
                    .collect()
            })
            .map(normalize_todo_drafts)
            .transpose()?;
        validate_item_content(
            &input.title,
            &input.content,
            todo_entries
                .as_ref()
                .is_some_and(|entries| !entries.is_empty()),
        )?;
        let title = normalize_title(&input.title);
        let tags_json = serde_json::to_string(&normalize_tags(input.tags))
            .map_err(|error| format!("serialize tags failed: {error}"))?;
        let (ciphertext, nonce) = self.encrypt_content(&input.id, &input.content)?;
        let now = now_ms();
        let new_revision = input.expected_revision + 1;

        let item = {
            let mut connection = self.lock()?;
            let transaction = connection
                .transaction()
                .map_err(|error| format!("start MCP item update failed: {error}"))?;
            let authorized = Self::find_mcp_writable_item(
                &transaction,
                client_id,
                &input.id,
                input.expected_revision,
                true,
            )?;
            let (kind, group) = authorized.ok_or_else(|| {
                "item is unavailable, read-only, or changed elsewhere; reload before saving"
                    .to_string()
            })?;
            let changed = transaction
                .execute(
                    r#"
                    UPDATE items
                    SET title = ?1,
                        content_ciphertext = ?2,
                        content_nonce = ?3,
                        tags_json = ?4,
                        due_at = ?5,
                        revision = revision + 1,
                        updated_at = ?6
                    WHERE id = ?7 AND revision = ?8 AND status <> 'archived'
                    "#,
                    params![
                        title,
                        ciphertext,
                        nonce,
                        tags_json,
                        input.due_at,
                        now,
                        input.id,
                        input.expected_revision,
                    ],
                )
                .map_err(|error| format!("update MCP item failed: {error}"))?;
            if changed == 0 {
                return Err("item changed elsewhere; reload before saving".to_string());
            }
            if let Some(entries) = todo_entries.as_ref() {
                if kind != ItemKind::Todo {
                    return Err("checklist entries are only valid for TODO items".to_string());
                }
                self.reconcile_todo_entries(&transaction, &input.id, entries, now)?;
                Self::sync_todo_parent_status(&transaction, &input.id, now, None, false)?;
            }
            Self::record_mcp_event(
                &transaction,
                &input.id,
                &group,
                client_id,
                "item.updated",
                new_revision,
                now,
            )?;
            let item = self.get_mcp_item_with_connection(&transaction, client_id, &input.id)?;
            transaction
                .commit()
                .map_err(|error| format!("commit MCP item update failed: {error}"))?;
            item
        };
        Ok(item)
    }

    pub fn mcp_move_item(
        &self,
        client_id: &str,
        input: McpMoveItemInput,
    ) -> Result<McpItemView, String> {
        let target_group = normalize_group(Some(&input.target_group))?;
        let now = now_ms();
        let new_revision = input.expected_revision + 1;
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction()
            .map_err(|error| format!("start MCP item move failed: {error}"))?;
        let (_, source_group) = Self::find_mcp_writable_item(
            &transaction,
            client_id,
            &input.id,
            input.expected_revision,
            false,
        )?
        .ok_or_else(|| {
            "item is unavailable, read-only, or changed elsewhere; reload before moving".to_string()
        })?;
        if source_group == target_group {
            return Err("item already belongs to the target group".to_string());
        }
        Self::assert_group_exists(&transaction, &target_group)?;
        Self::assert_mcp_group_write(&transaction, client_id, &target_group)?;

        let changed = transaction
            .execute(
                r#"
                UPDATE items
                SET group_key = ?1,
                    revision = revision + 1,
                    updated_at = ?2
                WHERE id = ?3 AND revision = ?4
                "#,
                params![target_group, now, input.id, input.expected_revision],
            )
            .map_err(|error| format!("move MCP item failed: {error}"))?;
        if changed == 0 {
            return Err("item changed elsewhere; reload before moving".to_string());
        }

        let payload_json = serde_json::to_string(&serde_json::json!({
            "fromGroup": &source_group,
            "toGroup": &target_group,
        }))
        .map_err(|error| format!("serialize MCP item move event failed: {error}"))?;
        Self::record_mcp_event_with_payload(
            &transaction,
            &input.id,
            &target_group,
            client_id,
            "item.moved",
            new_revision,
            &payload_json,
            now,
        )?;
        let item = self.get_mcp_item_with_connection(&transaction, client_id, &input.id)?;
        transaction
            .commit()
            .map_err(|error| format!("commit MCP item move failed: {error}"))?;
        Ok(item)
    }

    pub fn mcp_set_status(
        &self,
        client_id: &str,
        input: McpSetStatusInput,
    ) -> Result<McpItemView, String> {
        let now = now_ms();
        let completed_at = (input.status == ItemStatus::Completed).then_some(now);
        let archived_at = (input.status == ItemStatus::Archived).then_some(now);
        let new_revision = input.expected_revision + 1;
        let item = {
            let mut connection = self.lock()?;
            let transaction = connection
                .transaction()
                .map_err(|error| format!("start MCP status update failed: {error}"))?;
            let (kind, group) = Self::find_mcp_writable_item(
                &transaction,
                client_id,
                &input.id,
                input.expected_revision,
                false,
            )?
            .ok_or_else(|| {
                "item is unavailable, read-only, or changed elsewhere; reload before changing status"
                    .to_string()
            })?;

            if kind == ItemKind::Todo
                && matches!(input.status, ItemStatus::Active | ItemStatus::Completed)
            {
                let is_completed = input.status == ItemStatus::Completed;
                transaction
                    .execute(
                        r#"
                        UPDATE todo_entries
                        SET is_completed = ?1,
                            completed_at = CASE WHEN ?1 = 1 THEN ?2 ELSE NULL END,
                            revision = revision + 1,
                            updated_at = ?2
                        WHERE item_id = ?3 AND is_completed <> ?1
                        "#,
                        params![bool_to_int(is_completed), now, input.id],
                    )
                    .map_err(|error| format!("sync MCP TODO entries failed: {error}"))?;
            }
            let changed = transaction
                .execute(
                    r#"
                    UPDATE items
                    SET status = ?1,
                        completed_at = ?2,
                        archived_at = ?3,
                        revision = revision + 1,
                        updated_at = ?4
                    WHERE id = ?5 AND revision = ?6
                    "#,
                    params![
                        input.status.as_str(),
                        completed_at,
                        archived_at,
                        now,
                        input.id,
                        input.expected_revision,
                    ],
                )
                .map_err(|error| format!("update MCP item status failed: {error}"))?;
            if changed == 0 {
                return Err("item changed elsewhere; reload before changing status".to_string());
            }
            Self::record_mcp_event(
                &transaction,
                &input.id,
                &group,
                client_id,
                "item.status_changed",
                new_revision,
                now,
            )?;
            let item = self.get_mcp_item_with_connection(&transaction, client_id, &input.id)?;
            transaction
                .commit()
                .map_err(|error| format!("commit MCP status update failed: {error}"))?;
            item
        };
        Ok(item)
    }

    pub fn mcp_set_todo_entry_completed(
        &self,
        client_id: &str,
        input: McpSetTodoEntryCompletedInput,
    ) -> Result<McpItemView, String> {
        let now = now_ms();
        let new_revision = input.expected_item_revision + 1;
        let item = {
            let mut connection = self.lock()?;
            let transaction = connection
                .transaction()
                .map_err(|error| format!("start MCP TODO status update failed: {error}"))?;
            let (kind, group) = Self::find_mcp_writable_item(
                &transaction,
                client_id,
                &input.item_id,
                input.expected_item_revision,
                true,
            )?
            .ok_or_else(|| {
                "TODO item is unavailable, read-only, or changed elsewhere".to_string()
            })?;
            if kind != ItemKind::Todo {
                return Err("checklist entries are only valid for TODO items".to_string());
            }
            let changed = transaction
                .execute(
                    r#"
                    UPDATE todo_entries
                    SET is_completed = ?1,
                        completed_at = CASE WHEN ?1 = 1 THEN ?2 ELSE NULL END,
                        revision = revision + 1,
                        updated_at = ?2
                    WHERE id = ?3 AND item_id = ?4 AND revision = ?5
                    "#,
                    params![
                        bool_to_int(input.is_completed),
                        now,
                        input.entry_id,
                        input.item_id,
                        input.expected_entry_revision,
                    ],
                )
                .map_err(|error| format!("update MCP TODO entry status failed: {error}"))?;
            if changed == 0 {
                return Err("TODO entry changed elsewhere; reload before updating".to_string());
            }
            Self::sync_todo_parent_status(
                &transaction,
                &input.item_id,
                now,
                Some(input.expected_item_revision),
                true,
            )?;
            Self::record_mcp_event(
                &transaction,
                &input.item_id,
                &group,
                client_id,
                "todo_entry.completed_changed",
                new_revision,
                now,
            )?;
            let item =
                self.get_mcp_item_with_connection(&transaction, client_id, &input.item_id)?;
            transaction
                .commit()
                .map_err(|error| format!("commit MCP TODO status update failed: {error}"))?;
            item
        };
        Ok(item)
    }

    pub fn list_items(&self, include_archived: bool) -> Result<Vec<Item>, String> {
        let connection = self.lock()?;
        let sql = if include_archived {
            format!(
                "{ITEM_SELECT} ORDER BY CASE i.status WHEN 'active' THEN 0 WHEN 'completed' THEN 1 ELSE 2 END, i.updated_at DESC"
            )
        } else {
            format!(
                "{ITEM_SELECT} WHERE i.status <> 'archived' ORDER BY CASE i.status WHEN 'active' THEN 0 ELSE 1 END, i.updated_at DESC"
            )
        };
        let mut statement = connection
            .prepare(&sql)
            .map_err(|error| format!("prepare item list failed: {error}"))?;
        let stored_items = {
            let rows = statement
                .query_map([], Self::read_stored_item)
                .map_err(|error| format!("query item list failed: {error}"))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|error| format!("read item failed: {error}"))?
        };
        drop(statement);

        let mut items = Vec::new();
        for stored in stored_items {
            let mut item = self.decrypt_item(stored)?;
            item.todo_entries = self.read_todo_entries(&connection, &item.id)?;
            items.push(item);
        }
        Ok(items)
    }

    pub fn get_item(&self, id: &str) -> Result<Item, String> {
        let connection = self.lock()?;
        let sql = format!("{ITEM_SELECT} WHERE i.id = ?1");
        let stored = connection
            .query_row(&sql, [id], Self::read_stored_item)
            .optional()
            .map_err(|error| format!("query item failed: {error}"))?
            .ok_or_else(|| "item not found".to_string())?;
        let mut item = self.decrypt_item(stored)?;
        item.todo_entries = self.read_todo_entries(&connection, id)?;
        Ok(item)
    }

    pub fn create_item(&self, input: CreateItemInput) -> Result<Item, String> {
        validate_optional_timestamp_ms("dueAt", input.due_at)?;
        validate_optional_timestamp_ms("expiresAt", input.expires_at)?;
        let todo_entries = normalize_initial_todo_entries(input.kind, input.todo_entries)?;
        validate_item_content(&input.title, &input.content, !todo_entries.is_empty())?;
        let now = now_ms();
        let id = uuid::Uuid::new_v4().to_string();
        let title = normalize_title(&input.title);
        let group = normalize_group(input.group.as_deref())?;
        let tags = normalize_tags(input.tags);
        let tags_json = serde_json::to_string(&tags)
            .map_err(|error| format!("serialize tags failed: {error}"))?;
        let color = input.color.unwrap_or(ItemColor::Sun);
        let sensitivity = if input.kind == ItemKind::Secret {
            Sensitivity::Sensitive
        } else {
            Sensitivity::Standard
        };
        let mcp_access = if input.kind == ItemKind::Secret {
            McpAccess::Deny
        } else {
            input.mcp_access.unwrap_or(McpAccess::Inherit)
        };
        let expires_at = if input.kind == ItemKind::Secret {
            Some(input.expires_at.unwrap_or(now + DEFAULT_SECRET_TTL_MS))
        } else {
            input.expires_at
        };
        let (ciphertext, nonce) = self.encrypt_content(&id, &input.content)?;

        {
            let mut connection = self.lock()?;
            let transaction = connection
                .transaction()
                .map_err(|error| format!("start item transaction failed: {error}"))?;
            Self::ensure_group(&transaction, &group, now)?;
            transaction
                .execute(
                    r#"
                    INSERT INTO items (
                        id, kind, group_key, title, content_ciphertext, content_nonce, content_version,
                        status, sensitivity, color, tags_json, due_at, expires_at,
                        completed_at, mcp_access, revision, created_at, updated_at, archived_at
                    ) VALUES (
                        ?1, ?2, ?3, ?4, ?5, ?6, 1,
                        'active', ?7, ?8, ?9, ?10, ?11,
                        NULL, ?12, 1, ?13, ?13, NULL
                    )
                    "#,
                    params![
                        id,
                        input.kind.as_str(),
                        group,
                        title,
                        ciphertext,
                        nonce,
                        sensitivity.as_str(),
                        color.as_str(),
                        tags_json,
                        input.due_at,
                        expires_at,
                        mcp_access.as_str(),
                        now,
                    ],
                )
                .map_err(|error| format!("insert item failed: {error}"))?;
            transaction
                .execute(
                    r#"
                    INSERT INTO item_windows (
                        item_id, width, height, scale_factor, always_on_top,
                        opacity, is_visible, updated_at
                    ) VALUES (?1, 340, 360, 1, 1, 1, 0, ?2)
                    "#,
                    params![id, now],
                )
                .map_err(|error| format!("insert window state failed: {error}"))?;

            for (position, text) in todo_entries.iter().enumerate() {
                let entry_id = uuid::Uuid::new_v4().to_string();
                let (ciphertext, nonce) = self.encrypt_todo_entry(&id, &entry_id, text)?;
                transaction
                    .execute(
                        r#"
                        INSERT INTO todo_entries (
                            id, item_id, text_ciphertext, text_nonce, text_version,
                            is_completed, position, revision, created_at, updated_at, completed_at
                        ) VALUES (?1, ?2, ?3, ?4, 1, 0, ?5, 1, ?6, ?6, NULL)
                        "#,
                        params![entry_id, id, ciphertext, nonce, position as i64, now],
                    )
                    .map_err(|error| format!("insert TODO entry failed: {error}"))?;
            }
            transaction
                .commit()
                .map_err(|error| format!("commit item failed: {error}"))?;
        }

        self.get_item(&id)
    }

    pub fn update_item(&self, input: UpdateItemInput) -> Result<Item, String> {
        validate_optional_timestamp_ms("dueAt", input.due_at)?;
        validate_optional_timestamp_ms("expiresAt", input.expires_at)?;
        let todo_entries = input.todo_entries.map(normalize_todo_drafts).transpose()?;
        validate_item_content(
            &input.title,
            &input.content,
            todo_entries
                .as_ref()
                .is_some_and(|entries| !entries.is_empty()),
        )?;
        let title = normalize_title(&input.title);
        let group = normalize_group(Some(&input.group))?;
        let tags = normalize_tags(input.tags);
        let tags_json = serde_json::to_string(&tags)
            .map_err(|error| format!("serialize tags failed: {error}"))?;
        let (ciphertext, nonce) = self.encrypt_content(&input.id, &input.content)?;
        let now = now_ms();

        {
            let mut connection = self.lock()?;
            let transaction = connection
                .transaction()
                .map_err(|error| format!("start item update failed: {error}"))?;
            Self::ensure_group(&transaction, &group, now)?;
            let changed = transaction
                .execute(
                    r#"
                    UPDATE items
                    SET group_key = ?1,
                        title = ?2,
                        content_ciphertext = ?3,
                        content_nonce = ?4,
                        color = ?5,
                        tags_json = ?6,
                        due_at = ?7,
                        expires_at = ?8,
                        revision = revision + 1,
                        updated_at = ?9
                    WHERE id = ?10 AND revision = ?11 AND status <> 'archived'
                    "#,
                    params![
                        group,
                        title,
                        ciphertext,
                        nonce,
                        input.color.as_str(),
                        tags_json,
                        input.due_at,
                        input.expires_at,
                        now,
                        input.id,
                        input.expected_revision,
                    ],
                )
                .map_err(|error| format!("update item failed: {error}"))?;

            if changed == 0 {
                return Err("item changed elsewhere; reload before saving".to_string());
            }

            if let Some(entries) = todo_entries.as_ref() {
                let kind: String = transaction
                    .query_row("SELECT kind FROM items WHERE id = ?1", [&input.id], |row| {
                        row.get(0)
                    })
                    .map_err(|error| format!("read item kind failed: {error}"))?;
                if ItemKind::from_db(&kind)? != ItemKind::Todo {
                    return Err("checklist entries are only valid for TODO items".to_string());
                }
                self.reconcile_todo_entries(&transaction, &input.id, entries, now)?;
                Self::sync_todo_parent_status(&transaction, &input.id, now, None, false)?;
            }

            transaction
                .commit()
                .map_err(|error| format!("commit item update failed: {error}"))?;
        }
        self.get_item(&input.id)
    }

    pub fn set_status(&self, input: SetStatusInput) -> Result<Item, String> {
        let now = now_ms();
        let completed_at = (input.status == ItemStatus::Completed).then_some(now);
        let archived_at = (input.status == ItemStatus::Archived).then_some(now);
        {
            let mut connection = self.lock()?;
            let transaction = connection
                .transaction()
                .map_err(|error| format!("start item status update failed: {error}"))?;
            let kind = transaction
                .query_row(
                    "SELECT kind FROM items WHERE id = ?1 AND revision = ?2",
                    params![&input.id, input.expected_revision],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|error| format!("read item before status update failed: {error}"))?
                .ok_or_else(|| {
                    "item changed elsewhere; reload before changing status".to_string()
                })?;

            if kind == ItemKind::Todo.as_str()
                && matches!(input.status, ItemStatus::Active | ItemStatus::Completed)
            {
                let is_completed = input.status == ItemStatus::Completed;
                transaction
                    .execute(
                        r#"
                        UPDATE todo_entries
                        SET is_completed = ?1,
                            completed_at = CASE WHEN ?1 = 1 THEN ?2 ELSE NULL END,
                            revision = revision + 1,
                            updated_at = ?2
                        WHERE item_id = ?3 AND is_completed <> ?1
                        "#,
                        params![bool_to_int(is_completed), now, &input.id],
                    )
                    .map_err(|error| format!("sync TODO entries with parent failed: {error}"))?;
            }

            let changed = transaction
                .execute(
                    r#"
                UPDATE items
                SET status = ?1,
                    completed_at = ?2,
                    archived_at = ?3,
                    revision = revision + 1,
                    updated_at = ?4
                WHERE id = ?5 AND revision = ?6
                "#,
                    params![
                        input.status.as_str(),
                        completed_at,
                        archived_at,
                        now,
                        &input.id,
                        input.expected_revision,
                    ],
                )
                .map_err(|error| format!("update item status failed: {error}"))?;

            if changed == 0 {
                return Err("item changed elsewhere; reload before changing status".to_string());
            }
            transaction
                .commit()
                .map_err(|error| format!("commit item status update failed: {error}"))?;
        }
        self.get_item(&input.id)
    }

    pub fn add_todo_entry(&self, input: AddTodoEntryInput) -> Result<Item, String> {
        let text = normalize_todo_entry_text(&input.text)?;
        let now = now_ms();
        let entry_id = uuid::Uuid::new_v4().to_string();
        let (ciphertext, nonce) = self.encrypt_todo_entry(&input.item_id, &entry_id, &text)?;

        {
            let mut connection = self.lock()?;
            let transaction = connection
                .transaction()
                .map_err(|error| format!("start TODO entry creation failed: {error}"))?;
            Self::assert_todo_parent(&transaction, &input.item_id, input.expected_item_revision)?;
            let count: i64 = transaction
                .query_row(
                    "SELECT COUNT(*) FROM todo_entries WHERE item_id = ?1",
                    [&input.item_id],
                    |row| row.get(0),
                )
                .map_err(|error| format!("count TODO entries failed: {error}"))?;
            if count >= MAX_TODO_ENTRIES as i64 {
                return Err(format!(
                    "a TODO list can contain at most {MAX_TODO_ENTRIES} entries"
                ));
            }
            let position: i64 = transaction
                .query_row(
                    "SELECT COALESCE(MAX(position) + 1, 0) FROM todo_entries WHERE item_id = ?1",
                    [&input.item_id],
                    |row| row.get(0),
                )
                .map_err(|error| format!("choose TODO entry position failed: {error}"))?;
            transaction
                .execute(
                    r#"
                    INSERT INTO todo_entries (
                        id, item_id, text_ciphertext, text_nonce, text_version,
                        is_completed, position, revision, created_at, updated_at, completed_at
                    ) VALUES (?1, ?2, ?3, ?4, 1, 0, ?5, 1, ?6, ?6, NULL)
                    "#,
                    params![entry_id, input.item_id, ciphertext, nonce, position, now],
                )
                .map_err(|error| format!("insert TODO entry failed: {error}"))?;
            Self::sync_todo_parent_status(
                &transaction,
                &input.item_id,
                now,
                Some(input.expected_item_revision),
                true,
            )?;
            transaction
                .commit()
                .map_err(|error| format!("commit TODO entry creation failed: {error}"))?;
        }
        self.get_item(&input.item_id)
    }

    pub fn update_todo_entry(&self, input: UpdateTodoEntryInput) -> Result<Item, String> {
        let text = normalize_todo_entry_text(&input.text)?;
        let (ciphertext, nonce) =
            self.encrypt_todo_entry(&input.item_id, &input.entry_id, &text)?;
        let now = now_ms();

        {
            let mut connection = self.lock()?;
            let transaction = connection
                .transaction()
                .map_err(|error| format!("start TODO entry update failed: {error}"))?;
            Self::assert_todo_parent(&transaction, &input.item_id, input.expected_item_revision)?;
            let changed = transaction
                .execute(
                    r#"
                    UPDATE todo_entries
                    SET text_ciphertext = ?1,
                        text_nonce = ?2,
                        revision = revision + 1,
                        updated_at = ?3
                    WHERE id = ?4 AND item_id = ?5 AND revision = ?6
                    "#,
                    params![
                        ciphertext,
                        nonce,
                        now,
                        input.entry_id,
                        input.item_id,
                        input.expected_entry_revision
                    ],
                )
                .map_err(|error| format!("update TODO entry failed: {error}"))?;
            if changed == 0 {
                return Err("TODO entry changed elsewhere; reload before saving".to_string());
            }
            Self::sync_todo_parent_status(
                &transaction,
                &input.item_id,
                now,
                Some(input.expected_item_revision),
                true,
            )?;
            transaction
                .commit()
                .map_err(|error| format!("commit TODO entry update failed: {error}"))?;
        }
        self.get_item(&input.item_id)
    }

    pub fn set_todo_entry_completed(
        &self,
        input: SetTodoEntryCompletedInput,
    ) -> Result<Item, String> {
        let now = now_ms();
        {
            let mut connection = self.lock()?;
            let transaction = connection
                .transaction()
                .map_err(|error| format!("start TODO entry status update failed: {error}"))?;
            Self::assert_todo_parent(&transaction, &input.item_id, input.expected_item_revision)?;
            let changed = transaction
                .execute(
                    r#"
                    UPDATE todo_entries
                    SET is_completed = ?1,
                        completed_at = CASE WHEN ?1 = 1 THEN ?2 ELSE NULL END,
                        revision = revision + 1,
                        updated_at = ?2
                    WHERE id = ?3 AND item_id = ?4 AND revision = ?5
                    "#,
                    params![
                        bool_to_int(input.is_completed),
                        now,
                        input.entry_id,
                        input.item_id,
                        input.expected_entry_revision
                    ],
                )
                .map_err(|error| format!("update TODO entry status failed: {error}"))?;
            if changed == 0 {
                return Err("TODO entry changed elsewhere; reload before updating".to_string());
            }
            Self::sync_todo_parent_status(
                &transaction,
                &input.item_id,
                now,
                Some(input.expected_item_revision),
                true,
            )?;
            transaction
                .commit()
                .map_err(|error| format!("commit TODO entry status failed: {error}"))?;
        }
        self.get_item(&input.item_id)
    }

    pub fn delete_todo_entry(&self, input: DeleteTodoEntryInput) -> Result<Item, String> {
        let now = now_ms();
        {
            let mut connection = self.lock()?;
            let transaction = connection
                .transaction()
                .map_err(|error| format!("start TODO entry deletion failed: {error}"))?;
            Self::assert_todo_parent(&transaction, &input.item_id, input.expected_item_revision)?;
            let changed = transaction
                .execute(
                    "DELETE FROM todo_entries WHERE id = ?1 AND item_id = ?2 AND revision = ?3",
                    params![input.entry_id, input.item_id, input.expected_entry_revision],
                )
                .map_err(|error| format!("delete TODO entry failed: {error}"))?;
            if changed == 0 {
                return Err("TODO entry changed elsewhere; reload before deleting".to_string());
            }
            Self::sync_todo_parent_status(
                &transaction,
                &input.item_id,
                now,
                Some(input.expected_item_revision),
                true,
            )?;
            transaction
                .commit()
                .map_err(|error| format!("commit TODO entry deletion failed: {error}"))?;
        }
        self.get_item(&input.item_id)
    }

    pub fn set_window_pin(&self, item_id: &str, pinned: bool) -> Result<WindowState, String> {
        let now = now_ms();
        let changed = self
            .lock()?
            .execute(
                "UPDATE item_windows SET always_on_top = ?1, updated_at = ?2 WHERE item_id = ?3",
                params![bool_to_int(pinned), now, item_id],
            )
            .map_err(|error| format!("save pin state failed: {error}"))?;
        if changed == 0 {
            return Err("window state not found".to_string());
        }
        self.get_window_state(item_id)
    }

    pub fn get_theme(&self) -> Result<ThemeMode, String> {
        let value = self
            .lock()?
            .query_row(
                "SELECT value_json FROM settings WHERE key = ?1",
                [THEME_SETTING_KEY],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| format!("read theme setting failed: {error}"))?;

        match value {
            Some(value) => serde_json::from_str(&value)
                .map_err(|error| format!("decode theme setting failed: {error}")),
            None => Ok(ThemeMode::Light),
        }
    }

    pub fn set_theme(&self, theme: ThemeMode) -> Result<ThemeMode, String> {
        let value = serde_json::to_string(&theme)
            .map_err(|error| format!("encode theme setting failed: {error}"))?;
        self.lock()?
            .execute(
                r#"
                INSERT INTO settings (key, value_json, updated_at)
                VALUES (?1, ?2, ?3)
                ON CONFLICT(key) DO UPDATE SET
                    value_json = excluded.value_json,
                    updated_at = excluded.updated_at
                "#,
                params![THEME_SETTING_KEY, value, now_ms()],
            )
            .map_err(|error| format!("save theme setting failed: {error}"))?;
        Ok(theme)
    }

    pub fn get_locale(&self) -> Result<LocalePreference, String> {
        let value = self
            .lock()?
            .query_row(
                "SELECT value_json FROM settings WHERE key = ?1",
                [LOCALE_SETTING_KEY],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| format!("read locale setting failed: {error}"))?;

        match value {
            Some(value) => serde_json::from_str(&value)
                .map_err(|error| format!("decode locale setting failed: {error}")),
            None => Ok(LocalePreference::System),
        }
    }

    pub fn set_locale(&self, locale: LocalePreference) -> Result<LocalePreference, String> {
        let value = serde_json::to_string(&locale)
            .map_err(|error| format!("encode locale setting failed: {error}"))?;
        self.lock()?
            .execute(
                r#"
                INSERT INTO settings (key, value_json, updated_at)
                VALUES (?1, ?2, ?3)
                ON CONFLICT(key) DO UPDATE SET
                    value_json = excluded.value_json,
                    updated_at = excluded.updated_at
                "#,
                params![LOCALE_SETTING_KEY, value, now_ms()],
            )
            .map_err(|error| format!("save locale setting failed: {error}"))?;
        Ok(locale)
    }

    pub fn set_window_opacity(&self, input: WindowOpacityInput) -> Result<WindowState, String> {
        if !input.background_opacity.is_finite() || !input.text_opacity.is_finite() {
            return Err("window opacity must be a finite number".to_string());
        }
        let background_opacity = input.background_opacity.clamp(0.1, 1.0);
        let text_opacity = input.text_opacity.clamp(0.2, 1.0);
        let changed = self
            .lock()?
            .execute(
                r#"
                UPDATE item_windows
                SET background_opacity = ?1,
                    text_opacity = ?2,
                    updated_at = ?3
                WHERE item_id = ?4
                "#,
                params![background_opacity, text_opacity, now_ms(), input.item_id,],
            )
            .map_err(|error| format!("save window opacity failed: {error}"))?;
        if changed == 0 {
            return Err("window state not found".to_string());
        }
        self.get_window_state(&input.item_id)
    }

    pub fn set_window_visible(&self, item_id: &str, visible: bool) -> Result<(), String> {
        self.lock()?
            .execute(
                "UPDATE item_windows SET is_visible = ?1, updated_at = ?2 WHERE item_id = ?3",
                params![bool_to_int(visible), now_ms(), item_id],
            )
            .map_err(|error| format!("save window visibility failed: {error}"))?;
        Ok(())
    }

    pub fn save_window_geometry(&self, input: WindowGeometryInput) -> Result<(), String> {
        let width = input.width.map(|value| value.max(240.0));
        let height = input.height.map(|value| value.max(180.0));
        self.lock()?
            .execute(
                r#"
                UPDATE item_windows
                SET x = COALESCE(?1, x),
                    y = COALESCE(?2, y),
                    width = COALESCE(?3, width),
                    height = COALESCE(?4, height),
                    monitor_name = COALESCE(?5, monitor_name),
                    scale_factor = ?6,
                    updated_at = ?7
                WHERE item_id = ?8
                "#,
                params![
                    input.x,
                    input.y,
                    width,
                    height,
                    input.monitor_name,
                    input.scale_factor.max(0.1),
                    now_ms(),
                    input.item_id,
                ],
            )
            .map_err(|error| format!("save window geometry failed: {error}"))?;
        Ok(())
    }

    pub fn get_window_state(&self, item_id: &str) -> Result<WindowState, String> {
        self.lock()?
            .query_row(
                r#"
                SELECT x, y, width, height, monitor_name, scale_factor,
                       always_on_top, background_opacity, text_opacity,
                       is_visible, updated_at
                FROM item_windows
                WHERE item_id = ?1
                "#,
                [item_id],
                |row| {
                    Ok(WindowState {
                        x: row.get(0)?,
                        y: row.get(1)?,
                        width: row.get(2)?,
                        height: row.get(3)?,
                        monitor_name: row.get(4)?,
                        scale_factor: row.get(5)?,
                        always_on_top: row.get::<_, i64>(6)? != 0,
                        background_opacity: row.get(7)?,
                        text_opacity: row.get(8)?,
                        is_visible: row.get::<_, i64>(9)? != 0,
                        updated_at: row.get(10)?,
                    })
                },
            )
            .optional()
            .map_err(|error| format!("query window state failed: {error}"))?
            .ok_or_else(|| "window state not found".to_string())
    }

    fn get_mcp_item_with_connection(
        &self,
        connection: &Connection,
        client_id: &str,
        item_id: &str,
    ) -> Result<McpItemView, String> {
        let sql = format!(
            r#"
            {MCP_ITEM_SELECT}
            WHERE i.id = ?1
              AND i.kind IN ('note', 'todo')
              AND i.sensitivity = 'standard'
              AND i.mcp_access <> 'deny'
              AND EXISTS (
                  SELECT 1
                  FROM mcp_group_grants g
                  JOIN mcp_clients c ON c.id = g.client_id
                  WHERE g.client_id = ?2
                    AND g.group_key = i.group_key
                    AND g.revoked_at IS NULL
                    AND c.revoked_at IS NULL
              )
            "#
        );
        let stored = connection
            .query_row(
                &sql,
                params![item_id, client_id],
                Self::read_stored_mcp_item,
            )
            .optional()
            .map_err(|error| format!("query MCP item failed: {error}"))?
            .ok_or_else(|| "MCP item not found or access denied".to_string())?;
        self.decrypt_mcp_item(connection, stored)
    }

    fn decrypt_mcp_item(
        &self,
        connection: &Connection,
        stored: StoredMcpItem,
    ) -> Result<McpItemView, String> {
        let key = Key::from(self.key);
        let cipher = XChaCha20Poly1305::new(&key);
        let nonce = XNonce::try_from(stored.nonce.as_slice())
            .map_err(|_| "invalid item nonce".to_string())?;
        let plaintext = cipher
            .decrypt(
                &nonce,
                Payload {
                    msg: stored.ciphertext.as_slice(),
                    aad: stored.id.as_bytes(),
                },
            )
            .map_err(|_| "decrypt item content failed".to_string())?;
        let content =
            String::from_utf8(plaintext).map_err(|_| "item content is not UTF-8".to_string())?;
        let tags = serde_json::from_str(&stored.tags_json)
            .map_err(|error| format!("parse tags failed: {error}"))?;
        let todo_entries = self.read_mcp_todo_entries(connection, &stored.id)?;
        Ok(McpItemView {
            id: stored.id,
            kind: ItemKind::from_db(&stored.kind)?,
            group: stored.group_key,
            title: stored.title,
            content,
            status: ItemStatus::from_db(&stored.status)?,
            tags,
            due_at: stored.due_at,
            completed_at: stored.completed_at,
            revision: stored.revision,
            created_at: stored.created_at,
            updated_at: stored.updated_at,
            archived_at: stored.archived_at,
            todo_entries,
        })
    }

    fn read_mcp_todo_entries(
        &self,
        connection: &Connection,
        item_id: &str,
    ) -> Result<Vec<McpTodoEntryView>, String> {
        let mut statement = connection
            .prepare(
                r#"
                SELECT id, text_ciphertext, text_nonce, is_completed,
                       position, revision, created_at, updated_at, completed_at
                FROM todo_entries
                WHERE item_id = ?1
                ORDER BY position, created_at, id
                "#,
            )
            .map_err(|error| format!("prepare MCP TODO entries failed: {error}"))?;
        let rows = statement
            .query_map([item_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                    row.get::<_, i64>(3)? != 0,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, Option<i64>>(8)?,
                ))
            })
            .map_err(|error| format!("query MCP TODO entries failed: {error}"))?;
        let mut entries = Vec::new();
        for row in rows {
            let (
                id,
                ciphertext,
                nonce,
                is_completed,
                position,
                revision,
                created_at,
                updated_at,
                completed_at,
            ) = row.map_err(|error| format!("read MCP TODO entry failed: {error}"))?;
            let text = self.decrypt_todo_entry(item_id, &id, &ciphertext, &nonce)?;
            entries.push(McpTodoEntryView {
                id,
                text,
                is_completed,
                position,
                revision,
                created_at,
                updated_at,
                completed_at,
            });
        }
        Ok(entries)
    }

    fn ensure_group(transaction: &Transaction<'_>, group: &str, now: i64) -> Result<(), String> {
        transaction
            .execute(
                r#"
                INSERT OR IGNORE INTO item_groups (group_key, created_at, updated_at)
                VALUES (?1, ?2, ?2)
                "#,
                params![group, now],
            )
            .map_err(|error| format!("ensure group exists failed: {error}"))?;
        Ok(())
    }

    fn insert_group(transaction: &Transaction<'_>, group: &str, now: i64) -> Result<(), String> {
        let changed = transaction
            .execute(
                r#"
                INSERT OR IGNORE INTO item_groups (group_key, created_at, updated_at)
                VALUES (?1, ?2, ?2)
                "#,
                params![group, now],
            )
            .map_err(|error| format!("create group failed: {error}"))?;
        if changed == 0 {
            return Err("group already exists".to_string());
        }
        Ok(())
    }

    fn assert_group_exists(transaction: &Transaction<'_>, group: &str) -> Result<(), String> {
        let exists: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM item_groups WHERE group_key = ?1)",
                [group],
                |row| row.get(0),
            )
            .map_err(|error| format!("check group existence failed: {error}"))?;
        if !exists {
            return Err("target group does not exist; create it first".to_string());
        }
        Ok(())
    }

    fn get_group_with_connection(
        connection: &Connection,
        group: &str,
    ) -> Result<ItemGroup, String> {
        connection
            .query_row(
                r#"
                SELECT
                    g.group_key,
                    COUNT(i.id),
                    COALESCE(SUM(CASE WHEN i.status <> 'archived' THEN 1 ELSE 0 END), 0),
                    g.created_at,
                    g.updated_at
                FROM item_groups g
                LEFT JOIN items i ON i.group_key = g.group_key
                WHERE g.group_key = ?1
                GROUP BY g.group_key, g.created_at, g.updated_at
                "#,
                [group],
                |row| {
                    Ok(ItemGroup {
                        key: row.get(0)?,
                        item_count: row.get(1)?,
                        visible_item_count: row.get(2)?,
                        created_at: row.get(3)?,
                        updated_at: row.get(4)?,
                    })
                },
            )
            .optional()
            .map_err(|error| format!("read group failed: {error}"))?
            .ok_or_else(|| "group not found".to_string())
    }

    fn rename_group_in_transaction(
        transaction: &Transaction<'_>,
        group: &str,
        new_group: &str,
        now: i64,
    ) -> Result<(), String> {
        if group == "inbox" {
            return Err("the inbox group cannot be renamed".to_string());
        }
        if group == new_group {
            return Err("the new group name must be different".to_string());
        }

        let source_exists: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM item_groups WHERE group_key = ?1)",
                [group],
                |row| row.get(0),
            )
            .map_err(|error| format!("validate source group failed: {error}"))?;
        if !source_exists {
            return Err("group not found".to_string());
        }
        let target_exists: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM item_groups WHERE group_key = ?1)",
                [new_group],
                |row| row.get(0),
            )
            .map_err(|error| format!("validate target group failed: {error}"))?;
        if target_exists {
            return Err("the target group already exists".to_string());
        }

        transaction
            .execute(
                "UPDATE item_groups SET group_key = ?1, updated_at = ?2 WHERE group_key = ?3",
                params![new_group, now, group],
            )
            .map_err(|error| format!("rename group failed: {error}"))?;
        transaction
            .execute(
                r#"
                UPDATE items
                SET group_key = ?1,
                    revision = revision + 1,
                    updated_at = ?2
                WHERE group_key = ?3
                "#,
                params![new_group, now, group],
            )
            .map_err(|error| format!("move items into renamed group failed: {error}"))?;
        transaction
            .execute(
                "UPDATE mcp_group_grants SET group_key = ?1 WHERE group_key = ?2",
                params![new_group, group],
            )
            .map_err(|error| format!("move MCP grants into renamed group failed: {error}"))?;
        transaction
            .execute(
                "UPDATE task_events SET group_key = ?1 WHERE group_key = ?2",
                params![new_group, group],
            )
            .map_err(|error| format!("move task events into renamed group failed: {error}"))?;
        Ok(())
    }

    fn assert_mcp_group_management(
        transaction: &Transaction<'_>,
        client_id: &str,
    ) -> Result<(), String> {
        let scopes_json = transaction
            .query_row(
                "SELECT scopes_json FROM mcp_clients WHERE id = ?1 AND revoked_at IS NULL",
                [client_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| format!("check MCP group-management capability failed: {error}"))?
            .ok_or_else(|| "active MCP client not found".to_string())?;
        if !mcp_scope_enabled(&scopes_json, MCP_GROUP_MANAGEMENT_SCOPE)? {
            return Err(
                "MCP client is not explicitly authorized to manage groups; enable it in the main window"
                    .to_string(),
            );
        }
        Ok(())
    }

    fn assert_mcp_group_coordinator(
        transaction: &Transaction<'_>,
        client_id: &str,
        group: &str,
    ) -> Result<(), String> {
        let allowed: bool = transaction
            .query_row(
                r#"
                SELECT EXISTS(
                    SELECT 1
                    FROM mcp_group_grants g
                    JOIN mcp_clients c ON c.id = g.client_id
                    WHERE g.client_id = ?1
                      AND g.group_key = ?2
                      AND g.role = 'coordinator'
                      AND g.revoked_at IS NULL
                      AND c.revoked_at IS NULL
                )
                "#,
                params![client_id, group],
                |row| row.get(0),
            )
            .map_err(|error| format!("check MCP group coordinator access failed: {error}"))?;
        if !allowed {
            return Err("MCP client is not a coordinator for this group".to_string());
        }
        Ok(())
    }

    fn assert_mcp_group_write(
        transaction: &Transaction<'_>,
        client_id: &str,
        group: &str,
    ) -> Result<(), String> {
        let allowed: bool = transaction
            .query_row(
                r#"
                SELECT EXISTS(
                    SELECT 1
                    FROM mcp_group_grants g
                    JOIN mcp_clients c ON c.id = g.client_id
                    WHERE g.client_id = ?1
                      AND g.group_key = ?2
                      AND g.role IN ('contributor', 'coordinator')
                      AND g.revoked_at IS NULL
                      AND c.revoked_at IS NULL
                )
                "#,
                params![client_id, group],
                |row| row.get(0),
            )
            .map_err(|error| format!("check MCP group access failed: {error}"))?;
        if !allowed {
            return Err("MCP client does not have write access to this group".to_string());
        }
        Ok(())
    }

    fn find_mcp_writable_item(
        transaction: &Transaction<'_>,
        client_id: &str,
        item_id: &str,
        expected_revision: i64,
        require_unarchived: bool,
    ) -> Result<Option<(ItemKind, String)>, String> {
        transaction
            .query_row(
                r#"
                SELECT i.kind, i.group_key
                FROM items i
                WHERE i.id = ?1
                  AND i.revision = ?2
                  AND (?3 = 0 OR i.status <> 'archived')
                  AND i.kind IN ('note', 'todo')
                  AND i.sensitivity = 'standard'
                  AND i.mcp_access IN ('inherit', 'read_write')
                  AND EXISTS (
                      SELECT 1
                      FROM mcp_group_grants g
                      JOIN mcp_clients c ON c.id = g.client_id
                      WHERE g.client_id = ?4
                        AND g.group_key = i.group_key
                        AND g.role IN ('contributor', 'coordinator')
                        AND g.revoked_at IS NULL
                        AND c.revoked_at IS NULL
                  )
                "#,
                params![
                    item_id,
                    expected_revision,
                    bool_to_int(require_unarchived),
                    client_id,
                ],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(|error| format!("authorize MCP item mutation failed: {error}"))?
            .map(|(kind, group)| Ok((ItemKind::from_db(&kind)?, group)))
            .transpose()
    }

    fn record_mcp_event(
        transaction: &Transaction<'_>,
        item_id: &str,
        group: &str,
        client_id: &str,
        action: &str,
        item_revision: i64,
        now: i64,
    ) -> Result<(), String> {
        Self::record_mcp_event_with_payload(
            transaction,
            item_id,
            group,
            client_id,
            action,
            item_revision,
            "{}",
            now,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn record_mcp_event_with_payload(
        transaction: &Transaction<'_>,
        item_id: &str,
        group: &str,
        client_id: &str,
        action: &str,
        item_revision: i64,
        payload_json: &str,
        now: i64,
    ) -> Result<(), String> {
        transaction
            .execute(
                r#"
                INSERT INTO task_events (
                    item_id, group_key, actor_client_id, action,
                    item_revision, payload_json, created_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                "#,
                params![
                    item_id,
                    group,
                    client_id,
                    action,
                    item_revision,
                    payload_json,
                    now,
                ],
            )
            .map_err(|error| format!("record MCP item event failed: {error}"))?;
        Ok(())
    }

    fn read_stored_mcp_item(row: &Row<'_>) -> rusqlite::Result<StoredMcpItem> {
        Ok(StoredMcpItem {
            id: row.get(0)?,
            kind: row.get(1)?,
            group_key: row.get(2)?,
            title: row.get(3)?,
            ciphertext: row.get(4)?,
            nonce: row.get(5)?,
            status: row.get(6)?,
            tags_json: row.get(7)?,
            due_at: row.get(8)?,
            completed_at: row.get(9)?,
            revision: row.get(10)?,
            created_at: row.get(11)?,
            updated_at: row.get(12)?,
            archived_at: row.get(13)?,
        })
    }

    fn read_todo_entries(
        &self,
        connection: &Connection,
        item_id: &str,
    ) -> Result<Vec<TodoEntry>, String> {
        let mut statement = connection
            .prepare(
                r#"
                SELECT id, item_id, text_ciphertext, text_nonce, is_completed,
                       position, revision, created_at, updated_at, completed_at
                FROM todo_entries
                WHERE item_id = ?1
                ORDER BY position, created_at, id
                "#,
            )
            .map_err(|error| format!("prepare TODO entries failed: {error}"))?;
        let rows = statement
            .query_map([item_id], |row| {
                Ok(StoredTodoEntry {
                    id: row.get(0)?,
                    item_id: row.get(1)?,
                    ciphertext: row.get(2)?,
                    nonce: row.get(3)?,
                    is_completed: row.get::<_, i64>(4)? != 0,
                    position: row.get(5)?,
                    revision: row.get(6)?,
                    created_at: row.get(7)?,
                    updated_at: row.get(8)?,
                    completed_at: row.get(9)?,
                })
            })
            .map_err(|error| format!("query TODO entries failed: {error}"))?;

        let mut entries = Vec::new();
        for row in rows {
            let stored = row.map_err(|error| format!("read TODO entry failed: {error}"))?;
            let text = self.decrypt_todo_entry(
                &stored.item_id,
                &stored.id,
                &stored.ciphertext,
                &stored.nonce,
            )?;
            entries.push(TodoEntry {
                id: stored.id,
                item_id: stored.item_id,
                text,
                is_completed: stored.is_completed,
                position: stored.position,
                revision: stored.revision,
                created_at: stored.created_at,
                updated_at: stored.updated_at,
                completed_at: stored.completed_at,
            });
        }
        Ok(entries)
    }

    fn reconcile_todo_entries(
        &self,
        transaction: &Transaction<'_>,
        item_id: &str,
        entries: &[TodoEntryDraftInput],
        now: i64,
    ) -> Result<(), String> {
        let existing = {
            let mut statement = transaction
                .prepare("SELECT id, revision FROM todo_entries WHERE item_id = ?1")
                .map_err(|error| format!("prepare existing TODO entries failed: {error}"))?;
            let rows = statement
                .query_map([item_id], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                })
                .map_err(|error| format!("query existing TODO entries failed: {error}"))?;
            rows.collect::<rusqlite::Result<HashMap<_, _>>>()
                .map_err(|error| format!("read existing TODO entries failed: {error}"))?
        };

        let mut remaining = existing;
        let mut seen = HashSet::new();
        for (position, entry) in entries.iter().enumerate() {
            if let Some(entry_id) = entry.id.as_ref() {
                if !seen.insert(entry_id.clone()) {
                    return Err("duplicate TODO entry id".to_string());
                }
                let current_revision = remaining.remove(entry_id).ok_or_else(|| {
                    "TODO entry no longer exists; reload before saving".to_string()
                })?;
                if entry.expected_revision != Some(current_revision) {
                    return Err("TODO entry changed elsewhere; reload before saving".to_string());
                }
                let (ciphertext, nonce) =
                    self.encrypt_todo_entry(item_id, entry_id, &entry.text)?;
                let changed = transaction
                    .execute(
                        r#"
                        UPDATE todo_entries
                        SET text_ciphertext = ?1,
                            text_nonce = ?2,
                            is_completed = ?3,
                            position = ?4,
                            revision = revision + 1,
                            updated_at = ?5,
                            completed_at = CASE
                                WHEN ?3 = 1 THEN COALESCE(completed_at, ?5)
                                ELSE NULL
                            END
                        WHERE id = ?6 AND item_id = ?7 AND revision = ?8
                        "#,
                        params![
                            ciphertext,
                            nonce,
                            bool_to_int(entry.is_completed),
                            position as i64,
                            now,
                            entry_id,
                            item_id,
                            current_revision,
                        ],
                    )
                    .map_err(|error| format!("save TODO entry failed: {error}"))?;
                if changed == 0 {
                    return Err("TODO entry changed elsewhere; reload before saving".to_string());
                }
            } else {
                if entry.expected_revision.is_some() {
                    return Err("new TODO entry cannot have a revision".to_string());
                }
                let entry_id = uuid::Uuid::new_v4().to_string();
                let (ciphertext, nonce) =
                    self.encrypt_todo_entry(item_id, &entry_id, &entry.text)?;
                transaction
                    .execute(
                        r#"
                        INSERT INTO todo_entries (
                            id, item_id, text_ciphertext, text_nonce, text_version,
                            is_completed, position, revision, created_at, updated_at, completed_at
                        ) VALUES (
                            ?1, ?2, ?3, ?4, 1, ?5, ?6, 1, ?7, ?7,
                            CASE WHEN ?5 = 1 THEN ?7 ELSE NULL END
                        )
                        "#,
                        params![
                            entry_id,
                            item_id,
                            ciphertext,
                            nonce,
                            bool_to_int(entry.is_completed),
                            position as i64,
                            now,
                        ],
                    )
                    .map_err(|error| format!("insert TODO entry failed: {error}"))?;
            }
        }

        for entry_id in remaining.keys() {
            transaction
                .execute(
                    "DELETE FROM todo_entries WHERE id = ?1 AND item_id = ?2",
                    params![entry_id, item_id],
                )
                .map_err(|error| format!("delete removed TODO entry failed: {error}"))?;
        }
        Ok(())
    }

    fn assert_todo_parent(
        transaction: &Transaction<'_>,
        item_id: &str,
        expected_revision: i64,
    ) -> Result<(), String> {
        let parent = transaction
            .query_row(
                "SELECT kind, status, revision FROM items WHERE id = ?1",
                [item_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| format!("read TODO parent failed: {error}"))?
            .ok_or_else(|| "TODO item not found".to_string())?;

        if ItemKind::from_db(&parent.0)? != ItemKind::Todo {
            return Err("checklist entries are only valid for TODO items".to_string());
        }
        if ItemStatus::from_db(&parent.1)? == ItemStatus::Archived {
            return Err("archived TODO items cannot be changed".to_string());
        }
        if parent.2 != expected_revision {
            return Err("item changed elsewhere; reload before editing checklist".to_string());
        }
        Ok(())
    }

    fn sync_todo_parent_status(
        transaction: &Transaction<'_>,
        item_id: &str,
        now: i64,
        expected_revision: Option<i64>,
        bump_revision: bool,
    ) -> Result<(), String> {
        let (total, completed): (i64, i64) = transaction
            .query_row(
                r#"
                SELECT COUNT(*), COALESCE(SUM(is_completed), 0)
                FROM todo_entries
                WHERE item_id = ?1
                "#,
                [item_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|error| format!("summarize TODO entries failed: {error}"))?;
        let status = if total > 0 && total == completed {
            ItemStatus::Completed
        } else {
            ItemStatus::Active
        };
        let is_completed = status == ItemStatus::Completed;

        let changed = if bump_revision {
            let expected_revision = expected_revision
                .ok_or_else(|| "expected item revision is required".to_string())?;
            transaction
                .execute(
                    r#"
                    UPDATE items
                    SET status = ?1,
                        completed_at = CASE
                            WHEN ?2 = 1 THEN COALESCE(completed_at, ?3)
                            ELSE NULL
                        END,
                        revision = revision + 1,
                        updated_at = ?3
                    WHERE id = ?4 AND revision = ?5
                      AND kind = 'todo' AND status <> 'archived'
                    "#,
                    params![
                        status.as_str(),
                        bool_to_int(is_completed),
                        now,
                        item_id,
                        expected_revision,
                    ],
                )
                .map_err(|error| format!("sync TODO parent failed: {error}"))?
        } else {
            transaction
                .execute(
                    r#"
                    UPDATE items
                    SET status = ?1,
                        completed_at = CASE
                            WHEN ?2 = 1 THEN COALESCE(completed_at, ?3)
                            ELSE NULL
                        END,
                        updated_at = ?3
                    WHERE id = ?4 AND kind = 'todo' AND status <> 'archived'
                    "#,
                    params![status.as_str(), bool_to_int(is_completed), now, item_id],
                )
                .map_err(|error| format!("sync TODO parent failed: {error}"))?
        };

        if changed == 0 {
            return Err("item changed elsewhere; reload before editing checklist".to_string());
        }
        Ok(())
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, String> {
        self.connection
            .lock()
            .map_err(|_| "database lock is poisoned".to_string())
    }

    fn encrypt_content(&self, item_id: &str, content: &str) -> Result<(Vec<u8>, Vec<u8>), String> {
        let key = Key::from(self.key);
        let cipher = XChaCha20Poly1305::new(&key);
        let nonce = XNonce::generate();
        let ciphertext = cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: content.as_bytes(),
                    aad: item_id.as_bytes(),
                },
            )
            .map_err(|_| "encrypt item content failed".to_string())?;
        Ok((ciphertext, nonce.to_vec()))
    }

    fn encrypt_todo_entry(
        &self,
        item_id: &str,
        entry_id: &str,
        text: &str,
    ) -> Result<(Vec<u8>, Vec<u8>), String> {
        let key = Key::from(self.key);
        let cipher = XChaCha20Poly1305::new(&key);
        let nonce = XNonce::generate();
        let aad = format!("todo-entry\0{item_id}\0{entry_id}");
        let ciphertext = cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: text.as_bytes(),
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| "encrypt TODO entry failed".to_string())?;
        Ok((ciphertext, nonce.to_vec()))
    }

    fn decrypt_todo_entry(
        &self,
        item_id: &str,
        entry_id: &str,
        ciphertext: &[u8],
        nonce: &[u8],
    ) -> Result<String, String> {
        let key = Key::from(self.key);
        let cipher = XChaCha20Poly1305::new(&key);
        let nonce = XNonce::try_from(nonce).map_err(|_| "invalid TODO entry nonce".to_string())?;
        let aad = format!("todo-entry\0{item_id}\0{entry_id}");
        let plaintext = cipher
            .decrypt(
                &nonce,
                Payload {
                    msg: ciphertext,
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| "decrypt TODO entry failed".to_string())?;
        String::from_utf8(plaintext).map_err(|_| "TODO entry is not UTF-8".to_string())
    }

    fn decrypt_item(&self, stored: StoredItem) -> Result<Item, String> {
        let key = Key::from(self.key);
        let cipher = XChaCha20Poly1305::new(&key);
        let nonce = XNonce::try_from(stored.nonce.as_slice())
            .map_err(|_| "invalid item nonce".to_string())?;
        let plaintext = cipher
            .decrypt(
                &nonce,
                Payload {
                    msg: stored.ciphertext.as_slice(),
                    aad: stored.id.as_bytes(),
                },
            )
            .map_err(|_| "decrypt item content failed".to_string())?;
        let content =
            String::from_utf8(plaintext).map_err(|_| "item content is not UTF-8".to_string())?;
        let tags: Vec<String> = serde_json::from_str(&stored.tags_json)
            .map_err(|error| format!("parse tags failed: {error}"))?;

        let window = match (stored.window_width, stored.window_height) {
            (Some(width), Some(height)) => Some(WindowState {
                x: stored.window_x,
                y: stored.window_y,
                width,
                height,
                monitor_name: stored.monitor_name,
                scale_factor: stored.scale_factor.unwrap_or(1.0),
                always_on_top: stored.always_on_top.unwrap_or(1) != 0,
                background_opacity: stored.background_opacity.unwrap_or(1.0),
                text_opacity: stored.text_opacity.unwrap_or(1.0),
                is_visible: stored.is_visible.unwrap_or(0) != 0,
                updated_at: stored.window_updated_at.unwrap_or(stored.updated_at),
            }),
            _ => None,
        };

        Ok(Item {
            id: stored.id,
            kind: ItemKind::from_db(&stored.kind)?,
            group: stored.group_key,
            title: stored.title,
            content,
            status: ItemStatus::from_db(&stored.status)?,
            sensitivity: Sensitivity::from_db(&stored.sensitivity)?,
            color: ItemColor::from_db(&stored.color)?,
            tags,
            due_at: stored.due_at,
            expires_at: stored.expires_at,
            completed_at: stored.completed_at,
            mcp_access: McpAccess::from_db(&stored.mcp_access)?,
            revision: stored.revision,
            created_at: stored.created_at,
            updated_at: stored.updated_at,
            archived_at: stored.archived_at,
            window,
            todo_entries: Vec::new(),
        })
    }

    fn read_stored_item(row: &Row<'_>) -> rusqlite::Result<StoredItem> {
        Ok(StoredItem {
            id: row.get(0)?,
            kind: row.get(1)?,
            group_key: row.get(2)?,
            title: row.get(3)?,
            ciphertext: row.get(4)?,
            nonce: row.get(5)?,
            status: row.get(6)?,
            sensitivity: row.get(7)?,
            color: row.get(8)?,
            tags_json: row.get(9)?,
            due_at: row.get(10)?,
            expires_at: row.get(11)?,
            completed_at: row.get(12)?,
            mcp_access: row.get(13)?,
            revision: row.get(14)?,
            created_at: row.get(15)?,
            updated_at: row.get(16)?,
            archived_at: row.get(17)?,
            window_x: row.get(18)?,
            window_y: row.get(19)?,
            window_width: row.get(20)?,
            window_height: row.get(21)?,
            monitor_name: row.get(22)?,
            scale_factor: row.get(23)?,
            always_on_top: row.get(24)?,
            background_opacity: row.get(25)?,
            text_opacity: row.get(26)?,
            is_visible: row.get(27)?,
            window_updated_at: row.get(28)?,
        })
    }
}

pub fn load_or_create_master_key() -> Result<[u8; 32], String> {
    let entry = keyring::Entry::new("app.tips.desktop", "content-master-key")
        .map_err(|error| format!("open system credential store failed: {error}"))?;

    match entry.get_secret() {
        Ok(secret) => {
            let key: [u8; 32] = secret
                .try_into()
                .map_err(|_| "stored master key has an invalid length".to_string())?;
            Ok(key)
        }
        Err(keyring::Error::NoEntry) => {
            let generated = Key::generate();
            entry
                .set_secret(generated.as_slice())
                .map_err(|error| format!("save master key failed: {error}"))?;
            let mut key = [0_u8; 32];
            key.copy_from_slice(generated.as_slice());
            Ok(key)
        }
        Err(error) => Err(format!("read master key failed: {error}")),
    }
}

fn validate_item_content(title: &str, content: &str, has_todo_entries: bool) -> Result<(), String> {
    if title.trim().is_empty() && content.trim().is_empty() && !has_todo_entries {
        return Err("title, content, and checklist cannot all be empty".to_string());
    }
    if content.len() > MAX_CONTENT_BYTES {
        return Err("item content is larger than 1 MiB".to_string());
    }
    Ok(())
}

fn validate_optional_timestamp_ms(label: &str, value: Option<i64>) -> Result<(), String> {
    if value.is_some_and(|timestamp| !(0..=MAX_TIMESTAMP_MS).contains(&timestamp)) {
        return Err(format!(
            "{label} must be a Unix timestamp in milliseconds between 0 and {MAX_TIMESTAMP_MS}"
        ));
    }
    Ok(())
}

fn normalize_initial_todo_entries(
    kind: ItemKind,
    entries: Vec<String>,
) -> Result<Vec<String>, String> {
    if entries.len() > MAX_TODO_ENTRIES {
        return Err(format!(
            "a TODO list can contain at most {MAX_TODO_ENTRIES} entries"
        ));
    }

    let mut normalized = Vec::new();
    for text in entries {
        if text.trim().is_empty() {
            continue;
        }
        normalized.push(normalize_todo_entry_text(&text)?);
    }
    if kind != ItemKind::Todo && !normalized.is_empty() {
        return Err("checklist entries are only valid for TODO items".to_string());
    }
    Ok(normalized)
}

fn normalize_todo_drafts(
    entries: Vec<TodoEntryDraftInput>,
) -> Result<Vec<TodoEntryDraftInput>, String> {
    if entries.len() > MAX_TODO_ENTRIES {
        return Err(format!(
            "a TODO list can contain at most {MAX_TODO_ENTRIES} entries"
        ));
    }

    entries
        .into_iter()
        .map(|mut entry| {
            entry.text = normalize_todo_entry_text(&entry.text)?;
            match (&entry.id, entry.expected_revision) {
                (Some(_), Some(_)) | (None, None) => Ok(entry),
                (Some(_), None) => Err("existing TODO entry is missing its revision".to_string()),
                (None, Some(_)) => Err("new TODO entry cannot have a revision".to_string()),
            }
        })
        .collect()
}

fn normalize_todo_entry_text(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("TODO entry text cannot be empty".to_string());
    }
    if value.chars().count() > MAX_TODO_ENTRY_CHARS {
        return Err(format!(
            "TODO entry text cannot exceed {MAX_TODO_ENTRY_CHARS} characters"
        ));
    }
    Ok(value.to_string())
}

fn normalize_group(value: Option<&str>) -> Result<String, String> {
    let value = value.unwrap_or("inbox").trim().to_lowercase();
    if value.is_empty() {
        return Err("group cannot be empty".to_string());
    }
    if value.chars().count() > MAX_GROUP_CHARS {
        return Err(format!("group cannot exceed {MAX_GROUP_CHARS} characters"));
    }
    if !value
        .chars()
        .all(|ch| ch.is_alphanumeric() || matches!(ch, '-' | '_' | '.' | '/'))
    {
        return Err("group must use letters, numbers, '-', '_', '.', or '/'".to_string());
    }
    if value
        .split('/')
        .any(|segment| segment.is_empty() || matches!(segment, "." | ".."))
    {
        return Err("group path contains an invalid segment".to_string());
    }
    Ok(value)
}

fn normalize_title(value: &str) -> String {
    value.trim().chars().take(MAX_TITLE_CHARS).collect()
}

fn normalize_tags(tags: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    tags.into_iter()
        .map(|tag| tag.trim().chars().take(32).collect::<String>())
        .filter(|tag| !tag.is_empty())
        .filter(|tag| seen.insert(tag.to_lowercase()))
        .take(20)
        .collect()
}

fn bool_to_int(value: bool) -> i64 {
    if value { 1 } else { 0 }
}

fn hash_mcp_token(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

fn parse_mcp_scopes(value: &str) -> Result<Vec<String>, String> {
    serde_json::from_str(value)
        .map_err(|error| format!("decode MCP client capabilities failed: {error}"))
}

fn mcp_scope_enabled(value: &str, scope: &str) -> Result<bool, String> {
    Ok(parse_mcp_scopes(value)?
        .iter()
        .any(|candidate| candidate == scope))
}

fn parse_mcp_client_id(token: &str) -> Option<String> {
    let (client_id, secret) = token.strip_prefix("tips_")?.split_once('_')?;
    if secret.len() != 64 || !secret.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let parsed = uuid::Uuid::parse_str(client_id).ok()?;
    (parsed.to_string() == client_id).then(|| client_id.to_string())
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        Store::from_connection(Connection::open_in_memory().unwrap(), [7_u8; 32]).unwrap()
    }

    fn note_input(content: &str) -> CreateItemInput {
        CreateItemInput {
            kind: ItemKind::Note,
            group: None,
            title: "First note".to_string(),
            content: content.to_string(),
            color: Some(ItemColor::Mint),
            tags: vec!["work".to_string(), " Work ".to_string()],
            due_at: None,
            expires_at: None,
            mcp_access: None,
            todo_entries: vec![],
        }
    }

    #[test]
    fn persisted_timestamps_stay_within_the_supported_date_range() {
        assert!(validate_optional_timestamp_ms("dueAt", None).is_ok());
        assert!(validate_optional_timestamp_ms("dueAt", Some(0)).is_ok());
        assert!(validate_optional_timestamp_ms("dueAt", Some(MAX_TIMESTAMP_MS)).is_ok());
        assert!(validate_optional_timestamp_ms("dueAt", Some(-1)).is_err());
        assert!(validate_optional_timestamp_ms("dueAt", Some(MAX_TIMESTAMP_MS + 1)).is_err());
    }

    fn mcp_client_with_role(
        store: &Store,
        display_name: &str,
        group: &str,
        role: McpGroupRole,
    ) -> CreatedMcpClient {
        let created = store
            .create_mcp_client(CreateMcpClientInput {
                display_name: display_name.to_string(),
                transport: McpTransport::LoopbackHttp,
            })
            .unwrap();
        store
            .set_mcp_group_grant(SetMcpGroupGrantInput {
                client_id: created.client.id.clone(),
                group: group.to_string(),
                role,
            })
            .unwrap();
        created
    }

    #[test]
    fn content_is_encrypted_at_rest() {
        let store = store();
        let item = store
            .create_item(note_input("plain text must not leak"))
            .unwrap();
        let connection = store.connection.lock().unwrap();
        let ciphertext: Vec<u8> = connection
            .query_row(
                "SELECT content_ciphertext FROM items WHERE id = ?1",
                [item.id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!String::from_utf8_lossy(&ciphertext).contains("plain text must not leak"));
    }

    #[test]
    fn secret_is_denied_to_mcp_and_gets_default_expiry() {
        let store = store();
        let item = store
            .create_item(CreateItemInput {
                kind: ItemKind::Secret,
                group: None,
                title: "Temporary code".to_string(),
                content: "123456".to_string(),
                color: None,
                tags: vec![],
                due_at: None,
                expires_at: None,
                mcp_access: Some(McpAccess::ReadWrite),
                todo_entries: vec![],
            })
            .unwrap();

        assert_eq!(item.mcp_access, McpAccess::Deny);
        assert_eq!(item.sensitivity, Sensitivity::Sensitive);
        assert!(item.expires_at.is_some());
    }

    #[test]
    fn group_is_normalized_validated_and_updatable() {
        let store = store();
        let mut input = note_input("grouped item");
        input.group = Some(" Project-X/Backend ".to_string());
        let item = store.create_item(input).unwrap();
        assert_eq!(item.group, "project-x/backend");

        let updated = store
            .update_item(UpdateItemInput {
                id: item.id.clone(),
                expected_revision: item.revision,
                group: "OPS/Release".to_string(),
                title: item.title.clone(),
                content: item.content.clone(),
                color: item.color,
                tags: item.tags.clone(),
                due_at: item.due_at,
                expires_at: item.expires_at,
                todo_entries: None,
            })
            .unwrap();
        assert_eq!(updated.group, "ops/release");

        let mut invalid = note_input("invalid group");
        invalid.group = Some("project//backend".to_string());
        let error = store.create_item(invalid).unwrap_err();
        assert!(error.contains("invalid segment"));
    }

    #[test]
    fn v1_database_migrates_through_persistent_groups() {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(INITIAL_MIGRATION).unwrap();
        connection
            .pragma_update(None, "user_version", 1_i64)
            .unwrap();
        connection
            .execute(
                r#"
                INSERT INTO items (
                    id, kind, title, content_ciphertext, content_nonce,
                    status, sensitivity, color, tags_json, mcp_access,
                    revision, created_at, updated_at
                ) VALUES (
                    'legacy', 'note', '', X'00', zeroblob(24),
                    'active', 'standard', 'sun', '[]', 'inherit',
                    1, 1, 1
                )
                "#,
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO item_windows (item_id, opacity, updated_at) VALUES ('legacy', 0.55, 1)",
                [],
            )
            .unwrap();

        let store = Store::from_connection(connection, [7_u8; 32]).unwrap();
        let mcp_config = store.get_mcp_server_config().unwrap();
        let connection = store.connection.lock().unwrap();
        let (version, background, text, group): (i64, f64, f64, String) = connection
            .query_row(
                "SELECT (SELECT user_version FROM pragma_user_version), background_opacity, text_opacity, (SELECT group_key FROM items WHERE id = 'legacy') FROM item_windows WHERE item_id = 'legacy'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(version, 6);
        assert_eq!(background, 0.55);
        assert_eq!(text, 1.0);
        assert_eq!(group, "inbox");
        assert!(mcp_config.enabled);
        assert_eq!(mcp_config.bind_address, "127.0.0.1");
        assert!(!mcp_config.allow_lan);
        drop(connection);
        assert_eq!(store.list_groups().unwrap()[0].key, "inbox");
    }

    #[test]
    fn groups_persist_when_empty_and_rename_items_and_grants_atomically() {
        let store = store();
        let created_group = store
            .create_group(CreateGroupInput {
                group: "Project/Alpha".to_string(),
            })
            .unwrap();
        assert_eq!(created_group.key, "project/alpha");
        assert_eq!(created_group.item_count, 0);

        let mut input = note_input("grouped item");
        input.group = Some(created_group.key.clone());
        let item = store.create_item(input).unwrap();
        let client = mcp_client_with_role(
            &store,
            "Coordinator",
            &created_group.key,
            McpGroupRole::Coordinator,
        );

        let renamed = store
            .rename_group(RenameGroupInput {
                group: created_group.key,
                new_group: "Project/Beta".to_string(),
            })
            .unwrap();
        assert_eq!(renamed.key, "project/beta");
        assert_eq!(renamed.item_count, 1);
        let moved_item = store.get_item(&item.id).unwrap();
        assert_eq!(moved_item.group, "project/beta");
        assert_eq!(moved_item.revision, item.revision + 1);
        assert_eq!(
            store.mcp_list_groups(&client.client.id).unwrap(),
            vec!["project/beta".to_string()]
        );
        assert!(
            store
                .rename_group(RenameGroupInput {
                    group: "inbox".to_string(),
                    new_group: "renamed-inbox".to_string(),
                })
                .is_err()
        );
    }

    #[test]
    fn mcp_group_management_requires_explicit_client_capability() {
        let store = store();
        let viewer = mcp_client_with_role(&store, "Viewer", "read-only", McpGroupRole::Viewer);
        assert!(
            store
                .mcp_create_group(
                    &viewer.client.id,
                    McpCreateGroupInput {
                        group: "denied".to_string(),
                    },
                )
                .is_err()
        );

        let coordinator =
            mcp_client_with_role(&store, "Coordinator", "managed", McpGroupRole::Coordinator);
        assert!(!coordinator.client.can_manage_groups);
        assert!(
            store
                .mcp_create_group(
                    &coordinator.client.id,
                    McpCreateGroupInput {
                        group: "still-denied".to_string(),
                    },
                )
                .is_err()
        );
        store
            .set_mcp_group_management(&coordinator.client.id, true)
            .unwrap();
        assert!(
            store
                .list_mcp_clients(false)
                .unwrap()
                .into_iter()
                .find(|client| client.id == coordinator.client.id)
                .unwrap()
                .can_manage_groups
        );
        let created = store
            .mcp_create_group(
                &coordinator.client.id,
                McpCreateGroupInput {
                    group: "Managed/New".to_string(),
                },
            )
            .unwrap();
        assert_eq!(created.key, "managed/new");
        let renamed = store
            .mcp_rename_group(
                &coordinator.client.id,
                McpRenameGroupInput {
                    group: created.key,
                    new_group: "Managed/Renamed".to_string(),
                },
            )
            .unwrap();
        assert_eq!(renamed.key, "managed/renamed");

        store
            .set_mcp_group_management(&coordinator.client.id, false)
            .unwrap();
        assert!(
            store
                .mcp_rename_group(
                    &coordinator.client.id,
                    McpRenameGroupInput {
                        group: "managed/renamed".to_string(),
                        new_group: "not-allowed".to_string(),
                    },
                )
                .is_err()
        );

        store
            .set_mcp_group_management(&viewer.client.id, true)
            .unwrap();
        let viewer_created = store
            .mcp_create_group(
                &viewer.client.id,
                McpCreateGroupInput {
                    group: "explicitly-allowed".to_string(),
                },
            )
            .unwrap();
        assert_eq!(viewer_created.key, "explicitly-allowed");
        assert_eq!(
            store
                .list_mcp_group_grants(Some(&viewer.client.id), false)
                .unwrap()
                .into_iter()
                .find(|grant| grant.group == "explicitly-allowed")
                .unwrap()
                .role,
            McpGroupRole::Coordinator
        );
    }

    #[test]
    fn mcp_item_move_requires_write_access_to_source_and_target() {
        let store = store();
        let writer = mcp_client_with_role(&store, "Writer", "source", McpGroupRole::Contributor);
        let created = store
            .mcp_create_item(
                &writer.client.id,
                McpCreateItemInput {
                    group: "source".to_string(),
                    kind: ItemKind::Note,
                    title: "Move me".to_string(),
                    content: "Shared plan".to_string(),
                    tags: vec![],
                    due_at: None,
                    todo_entries: vec![],
                },
            )
            .unwrap();
        store
            .create_group(CreateGroupInput {
                group: "target".to_string(),
            })
            .unwrap();

        assert!(
            store
                .mcp_move_item(
                    &writer.client.id,
                    McpMoveItemInput {
                        id: created.id.clone(),
                        expected_revision: created.revision,
                        target_group: "target".to_string(),
                    },
                )
                .is_err()
        );
        store
            .set_mcp_group_grant(SetMcpGroupGrantInput {
                client_id: writer.client.id.clone(),
                group: "target".to_string(),
                role: McpGroupRole::Contributor,
            })
            .unwrap();
        let moved = store
            .mcp_move_item(
                &writer.client.id,
                McpMoveItemInput {
                    id: created.id.clone(),
                    expected_revision: created.revision,
                    target_group: "target".to_string(),
                },
            )
            .unwrap();
        assert_eq!(moved.group, "target");
        assert_eq!(moved.revision, created.revision + 1);

        let (event_group, payload): (String, String) = store
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT group_key, payload_json FROM task_events WHERE item_id = ?1 AND action = 'item.moved'",
                [&created.id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(event_group, "target");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&payload).unwrap(),
            serde_json::json!({"fromGroup": "source", "toGroup": "target"})
        );
    }

    #[test]
    fn mcp_tokens_are_hashed_transport_bound_and_revocable() {
        let store = store();
        let created = store
            .create_mcp_client(CreateMcpClientInput {
                display_name: "Local agent".to_string(),
                transport: McpTransport::LoopbackHttp,
            })
            .unwrap();

        let stored_hash: Vec<u8> = store
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT token_hash FROM mcp_clients WHERE id = ?1",
                [&created.client.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored_hash.len(), 32);
        assert_ne!(stored_hash, created.bearer_token.as_bytes());

        let principal = store
            .authenticate_mcp_client(&created.bearer_token, McpTransport::LoopbackHttp)
            .unwrap();
        assert_eq!(principal.client_id, created.client.id);
        assert_eq!(principal.display_name, "Local agent");
        assert_eq!(principal.transport, McpTransport::LoopbackHttp);
        assert!(
            store
                .authenticate_mcp_client(&created.bearer_token, McpTransport::Stdio)
                .is_err()
        );

        let mut invalid = created.bearer_token.clone();
        let replacement = if invalid.ends_with('0') { '1' } else { '0' };
        invalid.pop();
        invalid.push(replacement);
        assert!(
            store
                .authenticate_mcp_client(&invalid, McpTransport::LoopbackHttp)
                .is_err()
        );

        let active = store.list_mcp_clients(false).unwrap();
        assert_eq!(active.len(), 1);
        assert!(active[0].last_used_at.is_some());
        assert!(active[0].revoked_at.is_none());
        store.revoke_mcp_client(&created.client.id).unwrap();
        assert!(store.list_mcp_clients(false).unwrap().is_empty());
        assert_eq!(store.list_mcp_clients(true).unwrap().len(), 1);
        assert!(
            store
                .authenticate_mcp_client(&created.bearer_token, McpTransport::LoopbackHttp)
                .is_err()
        );
    }

    #[test]
    fn mcp_group_listing_uses_only_active_grants_for_active_clients() {
        let store = store();
        let client = mcp_client_with_role(&store, "Reader", "team/zeta", McpGroupRole::Viewer);
        store
            .set_mcp_group_grant(SetMcpGroupGrantInput {
                client_id: client.client.id.clone(),
                group: "team/alpha".to_string(),
                role: McpGroupRole::Contributor,
            })
            .unwrap();
        assert_eq!(
            store.mcp_list_groups(&client.client.id).unwrap(),
            vec!["team/alpha".to_string(), "team/zeta".to_string()]
        );

        let updated_grant = store
            .set_mcp_group_grant(SetMcpGroupGrantInput {
                client_id: client.client.id.clone(),
                group: "team/alpha".to_string(),
                role: McpGroupRole::Coordinator,
            })
            .unwrap();
        assert_eq!(updated_grant.role, McpGroupRole::Coordinator);
        assert_eq!(
            store
                .list_mcp_group_grants(Some(&client.client.id), false)
                .unwrap()
                .into_iter()
                .find(|grant| grant.group == "team/alpha")
                .unwrap()
                .role,
            McpGroupRole::Coordinator
        );

        store
            .revoke_mcp_group_grant(&client.client.id, "team/alpha")
            .unwrap();
        assert_eq!(
            store.mcp_list_groups(&client.client.id).unwrap(),
            vec!["team/zeta".to_string()]
        );
        assert_eq!(
            store
                .list_mcp_group_grants(Some(&client.client.id), true)
                .unwrap()
                .len(),
            2
        );
        store.revoke_mcp_client(&client.client.id).unwrap();
        assert!(store.mcp_list_groups(&client.client.id).unwrap().is_empty());
    }

    #[test]
    fn mcp_reads_filter_before_decryption_and_omit_desktop_state() {
        let store = store();
        let client = mcp_client_with_role(&store, "Reader", "shared", McpGroupRole::Viewer);

        let mut visible_input = note_input("shared text");
        visible_input.group = Some("shared".to_string());
        let visible = store.create_item(visible_input).unwrap();

        let mut denied_input = note_input("must remain private");
        denied_input.group = Some("shared".to_string());
        denied_input.mcp_access = Some(McpAccess::Deny);
        let denied = store.create_item(denied_input).unwrap();

        let mut sensitive_input = note_input("also private");
        sensitive_input.group = Some("shared".to_string());
        let sensitive = store.create_item(sensitive_input).unwrap();

        let secret = store
            .create_item(CreateItemInput {
                kind: ItemKind::Secret,
                group: Some("shared".to_string()),
                title: "Password".to_string(),
                content: "do not expose".to_string(),
                color: None,
                tags: vec![],
                due_at: None,
                expires_at: None,
                mcp_access: Some(McpAccess::ReadWrite),
                todo_entries: vec![],
            })
            .unwrap();

        {
            let connection = store.connection.lock().unwrap();
            connection
                .execute(
                    "UPDATE items SET content_ciphertext = X'00' WHERE id = ?1",
                    [&denied.id],
                )
                .unwrap();
            connection
                .execute(
                    "UPDATE items SET sensitivity = 'sensitive', content_ciphertext = X'00' WHERE id = ?1",
                    [&sensitive.id],
                )
                .unwrap();
        }

        let listed = store
            .mcp_list_items(
                &client.client.id,
                McpListItemsInput {
                    group: "shared".to_string(),
                    kind: None,
                    status: None,
                    updated_after: None,
                    limit: None,
                },
            )
            .unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, visible.id);
        assert_eq!(listed[0].content, "shared text");
        for unavailable in [&denied.id, &sensitive.id, &secret.id] {
            assert!(store.mcp_get_item(&client.client.id, unavailable).is_err());
        }

        let serialized = serde_json::to_value(&listed[0]).unwrap();
        let object = serialized.as_object().unwrap();
        for forbidden in ["window", "color", "sensitivity", "mcpAccess", "expiresAt"] {
            assert!(!object.contains_key(forbidden));
        }
    }

    #[test]
    fn mcp_mutations_enforce_roles_item_access_and_revisions() {
        let store = store();
        let writer =
            mcp_client_with_role(&store, "Writer", "project/core", McpGroupRole::Contributor);
        let viewer = mcp_client_with_role(&store, "Viewer", "project/core", McpGroupRole::Viewer);

        assert!(
            store
                .mcp_create_item(
                    &viewer.client.id,
                    McpCreateItemInput {
                        group: "project/core".to_string(),
                        kind: ItemKind::Note,
                        title: "denied".to_string(),
                        content: "denied".to_string(),
                        tags: vec![],
                        due_at: None,
                        todo_entries: vec![],
                    },
                )
                .is_err()
        );
        assert!(
            store
                .mcp_create_item(
                    &writer.client.id,
                    McpCreateItemInput {
                        group: "project/core".to_string(),
                        kind: ItemKind::Secret,
                        title: "secret".to_string(),
                        content: "secret".to_string(),
                        tags: vec![],
                        due_at: None,
                        todo_entries: vec![],
                    },
                )
                .is_err()
        );

        let created = store
            .mcp_create_item(
                &writer.client.id,
                McpCreateItemInput {
                    group: "project/core".to_string(),
                    kind: ItemKind::Todo,
                    title: "Release".to_string(),
                    content: String::new(),
                    tags: vec!["release".to_string()],
                    due_at: None,
                    todo_entries: vec!["Run tests".to_string()],
                },
            )
            .unwrap();
        assert_eq!(created.todo_entries.len(), 1);
        let entry = &created.todo_entries[0];
        let completed = store
            .mcp_set_todo_entry_completed(
                &writer.client.id,
                McpSetTodoEntryCompletedInput {
                    item_id: created.id.clone(),
                    entry_id: entry.id.clone(),
                    expected_item_revision: created.revision,
                    expected_entry_revision: entry.revision,
                    is_completed: true,
                },
            )
            .unwrap();
        assert_eq!(completed.status, ItemStatus::Completed);
        assert_eq!(completed.revision, 2);

        let updated = store
            .mcp_update_item(
                &writer.client.id,
                McpUpdateItemInput {
                    id: completed.id.clone(),
                    expected_revision: completed.revision,
                    title: "Release now".to_string(),
                    content: "Ready".to_string(),
                    tags: vec!["release".to_string()],
                    due_at: None,
                    todo_entries: None,
                },
            )
            .unwrap();
        assert_eq!(updated.revision, 3);
        assert_eq!(updated.title, "Release now");
        assert!(
            store
                .mcp_set_status(
                    &writer.client.id,
                    McpSetStatusInput {
                        id: updated.id.clone(),
                        expected_revision: 2,
                        status: ItemStatus::Active,
                    },
                )
                .is_err()
        );

        let read_only = {
            let mut input = note_input("read only");
            input.group = Some("project/core".to_string());
            input.mcp_access = Some(McpAccess::Read);
            store.create_item(input).unwrap()
        };
        assert!(
            store
                .mcp_update_item(
                    &writer.client.id,
                    McpUpdateItemInput {
                        id: read_only.id.clone(),
                        expected_revision: read_only.revision,
                        title: "not allowed".to_string(),
                        content: "not allowed".to_string(),
                        tags: vec![],
                        due_at: None,
                        todo_entries: None,
                    },
                )
                .is_err()
        );

        let connection = store.connection.lock().unwrap();
        let events: Vec<(String, i64)> = {
            let mut statement = connection
                .prepare(
                    "SELECT action, item_revision FROM task_events WHERE item_id = ?1 ORDER BY event_id",
                )
                .unwrap();
            statement
                .query_map([&created.id], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap()
        };
        assert_eq!(
            events,
            vec![
                ("item.created".to_string(), 1),
                ("todo_entry.completed_changed".to_string(), 2),
                ("item.updated".to_string(), 3),
            ]
        );
    }

    #[test]
    fn mcp_mutations_roll_back_when_their_result_cannot_be_built() {
        let store = store();
        let writer = mcp_client_with_role(&store, "Writer", "shared", McpGroupRole::Contributor);
        let original = store
            .mcp_create_item(
                &writer.client.id,
                McpCreateItemInput {
                    group: "shared".to_string(),
                    kind: ItemKind::Todo,
                    title: "Original".to_string(),
                    content: String::new(),
                    tags: vec![],
                    due_at: None,
                    todo_entries: vec!["First".to_string()],
                },
            )
            .unwrap();
        let entry = original.todo_entries[0].clone();
        let original_ciphertext: Vec<u8> = store
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT content_ciphertext FROM items WHERE id = ?1",
                [&original.id],
                |row| row.get(0),
            )
            .unwrap();

        // Simulate authorization disappearing between the mutation and result construction. The
        // trigger runs inside the mutation transaction, so a correct implementation must fail the
        // result lookup before commit and roll back the item, event, and trigger side effect.
        store
            .connection
            .lock()
            .unwrap()
            .execute_batch(
                r#"
                CREATE TRIGGER test_hide_mcp_mutation_result
                AFTER INSERT ON task_events
                BEGIN
                    UPDATE items SET group_key = 'moved' WHERE id = NEW.item_id;
                END;
                "#,
            )
            .unwrap();

        let create_error = store
            .mcp_create_item(
                &writer.client.id,
                McpCreateItemInput {
                    group: "shared".to_string(),
                    kind: ItemKind::Note,
                    title: "Must roll back".to_string(),
                    content: "not committed".to_string(),
                    tags: vec![],
                    due_at: None,
                    todo_entries: vec![],
                },
            )
            .unwrap_err();
        assert!(create_error.contains("not found or access denied"));

        let update_error = store
            .mcp_update_item(
                &writer.client.id,
                McpUpdateItemInput {
                    id: original.id.clone(),
                    expected_revision: original.revision,
                    title: "Must roll back".to_string(),
                    content: "not committed".to_string(),
                    tags: vec![],
                    due_at: None,
                    todo_entries: None,
                },
            )
            .unwrap_err();
        assert!(update_error.contains("not found or access denied"));

        let status_error = store
            .mcp_set_status(
                &writer.client.id,
                McpSetStatusInput {
                    id: original.id.clone(),
                    expected_revision: original.revision,
                    status: ItemStatus::Archived,
                },
            )
            .unwrap_err();
        assert!(status_error.contains("not found or access denied"));

        let todo_error = store
            .mcp_set_todo_entry_completed(
                &writer.client.id,
                McpSetTodoEntryCompletedInput {
                    item_id: original.id.clone(),
                    entry_id: entry.id.clone(),
                    expected_item_revision: original.revision,
                    expected_entry_revision: entry.revision,
                    is_completed: true,
                },
            )
            .unwrap_err();
        assert!(todo_error.contains("not found or access denied"));

        let connection = store.connection.lock().unwrap();
        let item_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM items", [], |row| row.get(0))
            .unwrap();
        let event_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM task_events", [], |row| row.get(0))
            .unwrap();
        let (group, title, content_ciphertext, status, revision): (
            String,
            String,
            Vec<u8>,
            String,
            i64,
        ) = connection
            .query_row(
                r#"
                SELECT group_key, title, content_ciphertext, status, revision
                FROM items
                WHERE id = ?1
                "#,
                [&original.id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .unwrap();
        let (entry_completed, entry_revision): (i64, i64) = connection
            .query_row(
                "SELECT is_completed, revision FROM todo_entries WHERE id = ?1",
                [&entry.id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();

        assert_eq!(item_count, 1);
        assert_eq!(event_count, 1);
        assert_eq!(group, "shared");
        assert_eq!(title, "Original");
        assert_eq!(status, "active");
        assert_eq!(revision, 1);
        assert_eq!(entry_completed, 0);
        assert_eq!(entry_revision, 1);
        assert_eq!(content_ciphertext, original_ciphertext);
    }

    #[test]
    fn stale_revision_is_rejected() {
        let store = store();
        let item = store.create_item(note_input("v1")).unwrap();
        let updated = store
            .update_item(UpdateItemInput {
                id: item.id.clone(),
                expected_revision: item.revision,
                group: item.group.clone(),
                title: item.title.clone(),
                content: "v2".to_string(),
                color: item.color,
                tags: item.tags.clone(),
                due_at: None,
                expires_at: None,
                todo_entries: None,
            })
            .unwrap();
        assert_eq!(updated.revision, 2);

        let error = store
            .update_item(UpdateItemInput {
                id: item.id,
                expected_revision: 1,
                group: "inbox".to_string(),
                title: "stale".to_string(),
                content: "stale".to_string(),
                color: ItemColor::Sun,
                tags: vec![],
                due_at: None,
                expires_at: None,
                todo_entries: None,
            })
            .unwrap_err();
        assert!(error.contains("changed elsewhere"));
    }

    #[test]
    fn window_state_is_persisted() {
        let store = store();
        let item = store.create_item(note_input("window state")).unwrap();

        store
            .save_window_geometry(WindowGeometryInput {
                item_id: item.id.clone(),
                x: Some(96.0),
                y: Some(112.0),
                width: Some(360.0),
                height: Some(300.0),
                monitor_name: Some("test-monitor".to_string()),
                scale_factor: 1.25,
            })
            .unwrap();
        store.set_window_pin(&item.id, false).unwrap();
        store
            .set_window_opacity(WindowOpacityInput {
                item_id: item.id.clone(),
                background_opacity: 0.35,
                text_opacity: 0.72,
            })
            .unwrap();
        store.set_window_visible(&item.id, true).unwrap();

        let state = store.get_window_state(&item.id).unwrap();
        assert_eq!(state.x, Some(96.0));
        assert_eq!(state.y, Some(112.0));
        assert_eq!(state.width, 360.0);
        assert_eq!(state.height, 300.0);
        assert_eq!(state.monitor_name.as_deref(), Some("test-monitor"));
        assert_eq!(state.scale_factor, 1.25);
        assert!(!state.always_on_top);
        assert_eq!(state.background_opacity, 0.35);
        assert_eq!(state.text_opacity, 0.72);
        assert!(state.is_visible);
    }

    #[test]
    fn item_color_change_is_persisted() {
        let store = store();
        let item = store.create_item(note_input("color change")).unwrap();
        let updated = store
            .update_item(UpdateItemInput {
                id: item.id.clone(),
                expected_revision: item.revision,
                group: item.group.clone(),
                title: item.title,
                content: item.content,
                color: ItemColor::Sky,
                tags: item.tags,
                due_at: item.due_at,
                expires_at: item.expires_at,
                todo_entries: None,
            })
            .unwrap();

        assert_eq!(updated.color, ItemColor::Sky);
        assert_eq!(updated.revision, 2);
        let stored: String = store
            .connection
            .lock()
            .unwrap()
            .query_row("SELECT color FROM items WHERE id = ?1", [item.id], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(stored, "sky");
    }

    #[test]
    fn todo_entries_are_encrypted_and_drive_parent_status() {
        let store = store();
        let item = store
            .create_item(CreateItemInput {
                kind: ItemKind::Todo,
                group: Some("release/backend".to_string()),
                title: "Release checklist".to_string(),
                content: String::new(),
                color: Some(ItemColor::Sun),
                tags: vec!["release".to_string()],
                due_at: None,
                expires_at: None,
                mcp_access: None,
                todo_entries: vec!["Run tests".to_string(), "Ship build".to_string()],
            })
            .unwrap();

        assert_eq!(item.todo_entries.len(), 2);
        assert_eq!(item.status, ItemStatus::Active);
        let connection = store.connection.lock().unwrap();
        let ciphertext: Vec<u8> = connection
            .query_row(
                "SELECT text_ciphertext FROM todo_entries WHERE item_id = ?1 ORDER BY position LIMIT 1",
                [&item.id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!String::from_utf8_lossy(&ciphertext).contains("Run tests"));
        drop(connection);

        let first = &item.todo_entries[0];
        let after_first = store
            .set_todo_entry_completed(SetTodoEntryCompletedInput {
                item_id: item.id.clone(),
                entry_id: first.id.clone(),
                expected_item_revision: item.revision,
                expected_entry_revision: first.revision,
                is_completed: true,
            })
            .unwrap();
        assert_eq!(after_first.status, ItemStatus::Active);
        assert_eq!(after_first.revision, 2);

        let second = &after_first.todo_entries[1];
        let completed = store
            .set_todo_entry_completed(SetTodoEntryCompletedInput {
                item_id: after_first.id.clone(),
                entry_id: second.id.clone(),
                expected_item_revision: after_first.revision,
                expected_entry_revision: second.revision,
                is_completed: true,
            })
            .unwrap();
        assert_eq!(completed.status, ItemStatus::Completed);
        assert!(
            completed
                .todo_entries
                .iter()
                .all(|entry| entry.is_completed)
        );

        let reopened = store
            .set_status(SetStatusInput {
                id: completed.id.clone(),
                expected_revision: completed.revision,
                status: ItemStatus::Active,
            })
            .unwrap();
        assert_eq!(reopened.status, ItemStatus::Active);
        assert!(
            reopened
                .todo_entries
                .iter()
                .all(|entry| !entry.is_completed)
        );
    }

    #[test]
    fn todo_entry_stale_revision_is_rejected() {
        let store = store();
        let item = store
            .create_item(CreateItemInput {
                kind: ItemKind::Todo,
                group: None,
                title: "Concurrent list".to_string(),
                content: String::new(),
                color: None,
                tags: vec![],
                due_at: None,
                expires_at: None,
                mcp_access: None,
                todo_entries: vec!["Only entry".to_string()],
            })
            .unwrap();
        let entry = &item.todo_entries[0];
        let updated = store
            .update_todo_entry(UpdateTodoEntryInput {
                item_id: item.id.clone(),
                entry_id: entry.id.clone(),
                expected_item_revision: item.revision,
                expected_entry_revision: entry.revision,
                text: "Updated entry".to_string(),
            })
            .unwrap();

        let error = store
            .update_todo_entry(UpdateTodoEntryInput {
                item_id: updated.id,
                entry_id: entry.id.clone(),
                expected_item_revision: updated.revision,
                expected_entry_revision: entry.revision,
                text: "Stale update".to_string(),
            })
            .unwrap_err();
        assert!(error.contains("changed elsewhere"));
    }

    #[test]
    fn theme_defaults_to_light_and_is_persisted() {
        let store = store();
        assert_eq!(store.get_theme().unwrap(), ThemeMode::Light);

        assert_eq!(
            store.set_theme(ThemeMode::BlackGold).unwrap(),
            ThemeMode::BlackGold
        );
        assert_eq!(store.get_theme().unwrap(), ThemeMode::BlackGold);

        let connection = store.connection.lock().unwrap();
        let raw: String = connection
            .query_row(
                "SELECT value_json FROM settings WHERE key = ?1",
                [THEME_SETTING_KEY],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(raw, "\"black_gold\"");
    }

    #[test]
    fn locale_defaults_to_system_and_is_persisted() {
        let store = store();
        assert_eq!(store.get_locale().unwrap(), LocalePreference::System);

        assert_eq!(
            store
                .set_locale(LocalePreference::SimplifiedChinese)
                .unwrap(),
            LocalePreference::SimplifiedChinese
        );
        assert_eq!(
            store.get_locale().unwrap(),
            LocalePreference::SimplifiedChinese
        );

        let connection = store.connection.lock().unwrap();
        let raw: String = connection
            .query_row(
                "SELECT value_json FROM settings WHERE key = ?1",
                [LOCALE_SETTING_KEY],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(raw, "\"zh-CN\"");
    }
}
