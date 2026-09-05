use serde::{Deserialize, Serialize};

use crate::model::{Item, ItemColor, ItemGroup, ItemKind, ItemStatus, McpAccess, TodoEntry};

pub const SYNC_BUNDLE_FORMAT: &str = "tips-progress";
pub const SYNC_BUNDLE_VERSION: u32 = 1;
pub const MAX_SYNC_BUNDLE_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_SYNC_BUNDLE_GROUPS: usize = 2_000;
pub const MAX_SYNC_BUNDLE_ITEMS: usize = 10_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncBundle {
    pub format: String,
    pub version: u32,
    pub application_version: String,
    pub exported_at: i64,
    pub groups: Vec<SyncGroup>,
    pub items: Vec<SyncItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncGroup {
    pub key: String,
    pub created_at: i64,
    pub updated_at: i64,
}

impl From<ItemGroup> for SyncGroup {
    fn from(group: ItemGroup) -> Self {
        Self {
            key: group.key,
            created_at: group.created_at,
            updated_at: group.updated_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncItem {
    pub id: String,
    pub kind: ItemKind,
    pub group: String,
    pub title: String,
    pub content: String,
    pub status: ItemStatus,
    pub color: ItemColor,
    pub tags: Vec<String>,
    pub due_at: Option<i64>,
    pub expires_at: Option<i64>,
    pub completed_at: Option<i64>,
    pub mcp_access: McpAccess,
    pub revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub archived_at: Option<i64>,
    pub todo_entries: Vec<SyncTodoEntry>,
}

impl SyncItem {
    pub fn from_item(item: Item) -> Self {
        Self {
            id: item.id,
            kind: item.kind,
            group: item.group,
            title: item.title,
            content: item.content,
            status: item.status,
            color: item.color,
            tags: item.tags,
            due_at: item.due_at,
            expires_at: item.expires_at,
            completed_at: item.completed_at,
            mcp_access: item.mcp_access,
            revision: item.revision,
            created_at: item.created_at,
            updated_at: item.updated_at,
            archived_at: item.archived_at,
            todo_entries: item
                .todo_entries
                .into_iter()
                .map(SyncTodoEntry::from)
                .collect(),
        }
    }

    pub fn matches_item(&self, item: &Item) -> bool {
        self.id == item.id
            && self.kind == item.kind
            && self.group == item.group
            && self.title == item.title
            && self.content == item.content
            && self.status == item.status
            && self.color == item.color
            && self.tags == item.tags
            && self.due_at == item.due_at
            && self.expires_at == item.expires_at
            && self.completed_at == item.completed_at
            && self.mcp_access == item.mcp_access
            && self.archived_at == item.archived_at
            && self.todo_entries.len() == item.todo_entries.len()
            && self
                .todo_entries
                .iter()
                .zip(&item.todo_entries)
                .all(|(source, current)| source.matches_entry(current))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncTodoEntry {
    pub id: String,
    pub text: String,
    pub is_completed: bool,
    pub position: i64,
    pub revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub completed_at: Option<i64>,
}

impl SyncTodoEntry {
    fn matches_entry(&self, entry: &TodoEntry) -> bool {
        self.id == entry.id
            && self.text == entry.text
            && self.is_completed == entry.is_completed
            && self.position == entry.position
            && self.completed_at == entry.completed_at
    }
}

impl From<TodoEntry> for SyncTodoEntry {
    fn from(entry: TodoEntry) -> Self {
        Self {
            id: entry.id,
            text: entry.text,
            is_completed: entry.is_completed,
            position: entry.position,
            revision: entry.revision,
            created_at: entry.created_at,
            updated_at: entry.updated_at,
            completed_at: entry.completed_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncExportSummary {
    pub group_count: usize,
    pub item_count: usize,
    pub excluded_private_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncImportSummary {
    pub groups_added: usize,
    pub items_added: usize,
    pub items_updated: usize,
    pub items_unchanged: usize,
    pub newer_local_items: usize,
    pub conflicts: usize,
}

#[derive(Debug)]
pub struct SyncImportOutcome {
    pub summary: SyncImportSummary,
    pub changed_item_ids: Vec<String>,
}
