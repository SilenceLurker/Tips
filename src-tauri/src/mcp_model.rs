use serde::{Deserialize, Serialize};

use crate::model::{ItemKind, ItemStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpTransport {
    Stdio,
    LoopbackHttp,
    LanHttp,
}

impl McpTransport {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stdio => "stdio",
            Self::LoopbackHttp => "loopback_http",
            Self::LanHttp => "lan_http",
        }
    }

    pub fn from_db(value: &str) -> Result<Self, String> {
        match value {
            "stdio" => Ok(Self::Stdio),
            "loopback_http" => Ok(Self::LoopbackHttp),
            "lan_http" => Ok(Self::LanHttp),
            _ => Err(format!("unknown MCP transport: {value}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum McpGroupRole {
    Viewer,
    Contributor,
    Coordinator,
}

impl McpGroupRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Viewer => "viewer",
            Self::Contributor => "contributor",
            Self::Coordinator => "coordinator",
        }
    }

    pub fn from_db(value: &str) -> Result<Self, String> {
        match value {
            "viewer" => Ok(Self::Viewer),
            "contributor" => Ok(Self::Contributor),
            "coordinator" => Ok(Self::Coordinator),
            _ => Err(format!("unknown MCP group role: {value}")),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateMcpClientInput {
    pub display_name: String,
    pub transport: McpTransport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpClientView {
    pub id: String,
    pub display_name: String,
    pub transport: McpTransport,
    pub can_manage_groups: bool,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
    pub revoked_at: Option<i64>,
}

/// Returned only when a client is created. The bearer token is never persisted in plaintext.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatedMcpClient {
    pub client: McpClientView,
    pub bearer_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpPrincipal {
    pub client_id: String,
    pub display_name: String,
    pub transport: McpTransport,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetMcpGroupGrantInput {
    pub client_id: String,
    pub group: String,
    pub role: McpGroupRole,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpGroupGrantView {
    pub client_id: String,
    pub group: String,
    pub role: McpGroupRole,
    pub created_at: i64,
    pub revoked_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerConfig {
    pub enabled: bool,
    pub bind_address: String,
    pub port: u16,
    pub allow_lan: bool,
}

impl Default for McpServerConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            bind_address: "127.0.0.1".to_string(),
            port: 4784,
            allow_lan: false,
        }
    }
}

/// MCP-facing checklist entry. Deliberately contains no desktop/window state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpTodoEntryView {
    pub id: String,
    pub text: String,
    pub is_completed: bool,
    pub position: i64,
    pub revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub completed_at: Option<i64>,
}

/// MCP-facing item. Appearance, sensitivity policy, encryption metadata and window state stay
/// private to the desktop application.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpItemView {
    pub id: String,
    pub kind: ItemKind,
    pub group: String,
    pub title: String,
    pub content: String,
    pub status: ItemStatus,
    pub tags: Vec<String>,
    pub due_at: Option<i64>,
    pub completed_at: Option<i64>,
    pub revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub archived_at: Option<i64>,
    pub todo_entries: Vec<McpTodoEntryView>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpListItemsInput {
    pub group: String,
    pub kind: Option<ItemKind>,
    pub status: Option<ItemStatus>,
    pub updated_after: Option<i64>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpCreateItemInput {
    pub group: String,
    pub kind: ItemKind,
    pub title: String,
    pub content: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub due_at: Option<i64>,
    #[serde(default)]
    pub todo_entries: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpTodoEntryDraftInput {
    pub id: Option<String>,
    pub expected_revision: Option<i64>,
    pub text: String,
    pub is_completed: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpUpdateItemInput {
    pub id: String,
    pub expected_revision: i64,
    pub title: String,
    pub content: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub due_at: Option<i64>,
    #[serde(default)]
    pub todo_entries: Option<Vec<McpTodoEntryDraftInput>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpSetStatusInput {
    pub id: String,
    pub expected_revision: i64,
    pub status: ItemStatus,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpSetTodoEntryCompletedInput {
    pub item_id: String,
    pub entry_id: String,
    pub expected_item_revision: i64,
    pub expected_entry_revision: i64,
    pub is_completed: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpCreateGroupInput {
    pub group: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpRenameGroupInput {
    pub group: String,
    pub new_group: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpMoveItemInput {
    pub id: String,
    pub expected_revision: i64,
    pub target_group: String,
}
