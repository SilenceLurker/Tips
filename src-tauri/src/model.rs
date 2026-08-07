use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ItemKind {
    Note,
    Todo,
    Secret,
}

impl ItemKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Note => "note",
            Self::Todo => "todo",
            Self::Secret => "secret",
        }
    }

    pub fn from_db(value: &str) -> Result<Self, String> {
        match value {
            "note" => Ok(Self::Note),
            "todo" => Ok(Self::Todo),
            "secret" => Ok(Self::Secret),
            _ => Err(format!("unknown item kind: {value}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ItemStatus {
    Active,
    Completed,
    Archived,
}

impl ItemStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Completed => "completed",
            Self::Archived => "archived",
        }
    }

    pub fn from_db(value: &str) -> Result<Self, String> {
        match value {
            "active" => Ok(Self::Active),
            "completed" => Ok(Self::Completed),
            "archived" => Ok(Self::Archived),
            _ => Err(format!("unknown item status: {value}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Sensitivity {
    Standard,
    Sensitive,
}

impl Sensitivity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Sensitive => "sensitive",
        }
    }

    pub fn from_db(value: &str) -> Result<Self, String> {
        match value {
            "standard" => Ok(Self::Standard),
            "sensitive" => Ok(Self::Sensitive),
            _ => Err(format!("unknown sensitivity: {value}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpAccess {
    Inherit,
    Deny,
    Read,
    ReadWrite,
}

impl McpAccess {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Inherit => "inherit",
            Self::Deny => "deny",
            Self::Read => "read",
            Self::ReadWrite => "read_write",
        }
    }

    pub fn from_db(value: &str) -> Result<Self, String> {
        match value {
            "inherit" => Ok(Self::Inherit),
            "deny" => Ok(Self::Deny),
            "read" => Ok(Self::Read),
            "read_write" => Ok(Self::ReadWrite),
            _ => Err(format!("unknown MCP access: {value}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ItemColor {
    Sun,
    Mint,
    Sky,
    Rose,
    Lilac,
    Paper,
}

impl ItemColor {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sun => "sun",
            Self::Mint => "mint",
            Self::Sky => "sky",
            Self::Rose => "rose",
            Self::Lilac => "lilac",
            Self::Paper => "paper",
        }
    }

    pub fn from_db(value: &str) -> Result<Self, String> {
        match value {
            "sun" => Ok(Self::Sun),
            "mint" => Ok(Self::Mint),
            "sky" => Ok(Self::Sky),
            "rose" => Ok(Self::Rose),
            "lilac" => Ok(Self::Lilac),
            "paper" => Ok(Self::Paper),
            _ => Err(format!("unknown item color: {value}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode {
    Light,
    BlackGold,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowState {
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub width: f64,
    pub height: f64,
    pub monitor_name: Option<String>,
    pub scale_factor: f64,
    pub always_on_top: bool,
    pub background_opacity: f64,
    pub text_opacity: f64,
    pub is_visible: bool,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TodoEntry {
    pub id: String,
    pub item_id: String,
    pub text: String,
    pub is_completed: bool,
    pub position: i64,
    pub revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub completed_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub id: String,
    pub kind: ItemKind,
    pub group: String,
    pub title: String,
    pub content: String,
    pub status: ItemStatus,
    pub sensitivity: Sensitivity,
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
    pub window: Option<WindowState>,
    pub todo_entries: Vec<TodoEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateItemInput {
    pub kind: ItemKind,
    #[serde(default)]
    pub group: Option<String>,
    pub title: String,
    pub content: String,
    pub color: Option<ItemColor>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub due_at: Option<i64>,
    pub expires_at: Option<i64>,
    pub mcp_access: Option<McpAccess>,
    #[serde(default)]
    pub todo_entries: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TodoEntryDraftInput {
    pub id: Option<String>,
    pub expected_revision: Option<i64>,
    pub text: String,
    pub is_completed: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateItemInput {
    pub id: String,
    pub expected_revision: i64,
    pub group: String,
    pub title: String,
    pub content: String,
    pub color: ItemColor,
    #[serde(default)]
    pub tags: Vec<String>,
    pub due_at: Option<i64>,
    pub expires_at: Option<i64>,
    #[serde(default)]
    pub todo_entries: Option<Vec<TodoEntryDraftInput>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddTodoEntryInput {
    pub item_id: String,
    pub expected_item_revision: i64,
    pub text: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateTodoEntryInput {
    pub item_id: String,
    pub entry_id: String,
    pub expected_item_revision: i64,
    pub expected_entry_revision: i64,
    pub text: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetTodoEntryCompletedInput {
    pub item_id: String,
    pub entry_id: String,
    pub expected_item_revision: i64,
    pub expected_entry_revision: i64,
    pub is_completed: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteTodoEntryInput {
    pub item_id: String,
    pub entry_id: String,
    pub expected_item_revision: i64,
    pub expected_entry_revision: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetStatusInput {
    pub id: String,
    pub expected_revision: i64,
    pub status: ItemStatus,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowGeometryInput {
    pub item_id: String,
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub monitor_name: Option<String>,
    pub scale_factor: f64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowOpacityInput {
    pub item_id: String,
    pub background_opacity: f64,
    pub text_opacity: f64,
}
