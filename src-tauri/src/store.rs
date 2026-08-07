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

use crate::model::{
    AddTodoEntryInput, CreateItemInput, DeleteTodoEntryInput, Item, ItemColor, ItemKind,
    ItemStatus, McpAccess, Sensitivity, SetStatusInput, SetTodoEntryCompletedInput, ThemeMode,
    TodoEntry, TodoEntryDraftInput, UpdateItemInput, UpdateTodoEntryInput, WindowGeometryInput,
    WindowOpacityInput, WindowState,
};

const INITIAL_MIGRATION: &str = include_str!("../migrations/001_initial.sql");
const WINDOW_OPACITY_MIGRATION: &str = include_str!("../migrations/002_window_opacity.sql");
const TODO_ENTRIES_MIGRATION: &str = include_str!("../migrations/003_todo_entries.sql");
const ITEM_GROUPS_MIGRATION: &str = include_str!("../migrations/004_item_groups.sql");
const LATEST_SCHEMA_VERSION: i64 = 4;
const MAX_TITLE_CHARS: usize = 160;
const MAX_GROUP_CHARS: usize = 96;
const MAX_CONTENT_BYTES: usize = 1024 * 1024;
const MAX_TODO_ENTRIES: usize = 200;
const MAX_TODO_ENTRY_CHARS: usize = 500;
const DEFAULT_SECRET_TTL_MS: i64 = 24 * 60 * 60 * 1000;
const THEME_SETTING_KEY: &str = "appearance.theme";

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
    fn v1_database_migrates_window_opacity() {
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
        let connection = store.connection.lock().unwrap();
        let (version, background, text, group): (i64, f64, f64, String) = connection
            .query_row(
                "SELECT (SELECT user_version FROM pragma_user_version), background_opacity, text_opacity, (SELECT group_key FROM items WHERE id = 'legacy') FROM item_windows WHERE item_id = 'legacy'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(version, 4);
        assert_eq!(background, 0.55);
        assert_eq!(text, 1.0);
        assert_eq!(group, "inbox");
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
}
