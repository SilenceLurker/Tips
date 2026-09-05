use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{Arc, RwLock},
};

use axum::{
    Router,
    extract::{Request, State as AxumState},
    http::{
        StatusCode,
        header::{AUTHORIZATION, CACHE_CONTROL, RETRY_AFTER, WWW_AUTHENTICATE},
        request::Parts,
    },
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use rmcp::{
    ErrorData,
    handler::server::{tool::Extension, wrapper::Parameters},
    model::CallToolResult,
    schemars::{self, JsonSchema},
    tool, tool_handler, tool_router,
    transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    },
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::{
    mcp_model::{
        McpCreateGroupInput, McpCreateItemInput, McpItemView, McpListItemsInput, McpMoveItemInput,
        McpPrincipal, McpRenameGroupInput, McpServerConfig, McpSetStatusInput,
        McpSetTodoEntryCompletedInput, McpTodoEntryDraftInput, McpTransport, McpUpdateItemInput,
    },
    model::{GroupChangedEvent, ItemKind, ItemStatus},
    store::Store,
};

const MCP_MAX_REQUEST_BYTES: usize = 1024 * 1024;
const MCP_MAX_IN_FLIGHT_REQUESTS: usize = 16;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpRuntimeStatus {
    pub enabled: bool,
    pub running: bool,
    pub endpoint: String,
    pub error: Option<String>,
}

impl Default for McpRuntimeStatus {
    fn default() -> Self {
        Self {
            enabled: false,
            running: false,
            endpoint: "http://127.0.0.1:4784/mcp".to_string(),
            error: None,
        }
    }
}

#[derive(Default)]
pub struct McpRuntimeState {
    status: RwLock<McpRuntimeStatus>,
}

impl McpRuntimeState {
    pub fn snapshot(&self) -> McpRuntimeStatus {
        self.status
            .read()
            .map(|status| status.clone())
            .unwrap_or_else(|_| McpRuntimeStatus {
                error: Some("MCP runtime status lock is unavailable".to_string()),
                ..McpRuntimeStatus::default()
            })
    }

    fn replace(&self, status: McpRuntimeStatus) {
        if let Ok(mut current) = self.status.write() {
            *current = status;
        }
    }
}

pub fn endpoint_for(config: &McpServerConfig) -> String {
    let host = config
        .bind_address
        .parse::<IpAddr>()
        .map(|address| match address {
            IpAddr::V4(address) => address.to_string(),
            IpAddr::V6(address) => format!("[{address}]"),
        })
        .unwrap_or_else(|_| config.bind_address.clone());
    format!("http://{host}:{}/mcp", config.port)
}

pub fn spawn(app: AppHandle, config: McpServerConfig) {
    let endpoint = endpoint_for(&config);
    set_runtime_status(
        &app,
        McpRuntimeStatus {
            enabled: config.enabled,
            running: false,
            endpoint: endpoint.clone(),
            error: None,
        },
    );

    if !config.enabled {
        return;
    }

    tauri::async_runtime::spawn(async move {
        if let Err(error) = serve(app.clone(), config).await {
            set_runtime_status(
                &app,
                McpRuntimeStatus {
                    enabled: true,
                    running: false,
                    endpoint,
                    error: Some(error),
                },
            );
        }
    });
}

async fn serve(app: AppHandle, config: McpServerConfig) -> Result<(), String> {
    let bind_ip = config
        .bind_address
        .parse::<IpAddr>()
        .map_err(|_| "MCP bind address must be an IP address".to_string())?;
    if config.allow_lan || bind_ip != IpAddr::V4(Ipv4Addr::LOCALHOST) {
        return Err(
            "LAN MCP is not available in this version; bind address must remain 127.0.0.1"
                .to_string(),
        );
    }

    let socket_address = SocketAddr::new(bind_ip, config.port);
    let listener = tokio::net::TcpListener::bind(socket_address)
        .await
        .map_err(|error| format!("bind MCP endpoint {socket_address} failed: {error}"))?;

    let endpoint = endpoint_for(&config);
    let handler_app = app.clone();
    let service = StreamableHttpService::new(
        move || Ok(TipsMcpServer::new(handler_app.clone())),
        LocalSessionManager::default().into(),
        StreamableHttpServerConfig::default()
            .with_legacy_session_mode(false)
            .with_json_response(true)
            .with_max_request_body_bytes(MCP_MAX_REQUEST_BYTES)
            .with_allowed_hosts([
                format!("127.0.0.1:{}", config.port),
                format!("localhost:{}", config.port),
            ])
            .with_allowed_origins([
                format!("http://127.0.0.1:{}", config.port),
                format!("http://localhost:{}", config.port),
            ]),
    );
    let router = Router::new()
        .nest_service("/mcp", service)
        .layer(middleware::from_fn_with_state(
            AuthState {
                app: app.clone(),
                request_slots: Arc::new(tokio::sync::Semaphore::new(MCP_MAX_IN_FLIGHT_REQUESTS)),
            },
            require_bearer,
        ));

    set_runtime_status(
        &app,
        McpRuntimeStatus {
            enabled: true,
            running: true,
            endpoint: endpoint.clone(),
            error: None,
        },
    );

    axum::serve(listener, router)
        .await
        .map_err(|error| format!("MCP endpoint {endpoint} stopped: {error}"))
}

fn set_runtime_status(app: &AppHandle, status: McpRuntimeStatus) {
    app.state::<McpRuntimeState>().replace(status);
}

#[derive(Clone)]
struct AuthState {
    app: AppHandle,
    request_slots: Arc<tokio::sync::Semaphore>,
}

async fn require_bearer(
    AxumState(state): AxumState<AuthState>,
    mut request: Request,
    next: Next,
) -> Response {
    let bearer_token = request
        .headers_mut()
        .remove(AUTHORIZATION)
        .and_then(|value| value.to_str().ok().map(str::to_owned))
        .and_then(|value| parse_bearer(&value).map(str::to_owned));
    let Some(bearer_token) = bearer_token else {
        return unauthorized_response();
    };
    let Ok(_request_permit) = state.request_slots.clone().try_acquire_owned() else {
        return too_many_requests_response();
    };

    let auth_app = state.app.clone();
    let principal = match tauri::async_runtime::spawn_blocking(move || {
        auth_app
            .state::<Store>()
            .authenticate_mcp_client(&bearer_token, McpTransport::LoopbackHttp)
    })
    .await
    {
        Ok(Ok(principal)) => principal,
        _ => return unauthorized_response(),
    };

    request.extensions_mut().insert(principal);
    next.run(request).await
}

fn parse_bearer(value: &str) -> Option<&str> {
    let (scheme, token) = value.trim().split_once(' ')?;
    let token = token.trim();
    (scheme.eq_ignore_ascii_case("bearer")
        && !token.is_empty()
        && !token.contains(char::is_whitespace))
    .then_some(token)
}

fn unauthorized_response() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [
            (WWW_AUTHENTICATE, "Bearer realm=\"Tips MCP\""),
            (CACHE_CONTROL, "no-store"),
        ],
        "Unauthorized",
    )
        .into_response()
}

fn too_many_requests_response() -> Response {
    (
        StatusCode::TOO_MANY_REQUESTS,
        [(RETRY_AFTER, "1"), (CACHE_CONTROL, "no-store")],
        "Too Many Requests",
    )
        .into_response()
}

#[derive(Clone)]
struct TipsMcpServer {
    app: AppHandle,
}

impl TipsMcpServer {
    fn new(app: AppHandle) -> Self {
        Self { app }
    }

    async fn call_store<T, F>(&self, operation: F) -> Result<T, String>
    where
        T: Send + 'static,
        F: FnOnce(&Store) -> Result<T, String> + Send + 'static,
    {
        let app = self.app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let store = app.state::<Store>();
            operation(&store)
        })
        .await
        .map_err(|error| format!("MCP storage task failed: {error}"))?
    }

    fn principal(parts: &Parts) -> Result<McpPrincipal, ErrorData> {
        parts
            .extensions
            .get::<McpPrincipal>()
            .cloned()
            .ok_or_else(|| ErrorData::internal_error("authenticated MCP principal missing", None))
    }

    fn notify_items_changed(&self, item_id: &str) {
        let _ = self.app.emit("mcp-items-changed", item_id);
    }

    fn notify_group_changed(&self, previous_key: Option<String>, group: crate::model::ItemGroup) {
        let _ = self.app.emit(
            "groups-changed",
            GroupChangedEvent {
                previous_key,
                group,
            },
        );
    }
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum ItemKindParam {
    Note,
    Todo,
}

impl From<ItemKindParam> for ItemKind {
    fn from(value: ItemKindParam) -> Self {
        match value {
            ItemKindParam::Note => Self::Note,
            ItemKindParam::Todo => Self::Todo,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum ItemStatusParam {
    Active,
    Completed,
    Archived,
}

impl From<ItemStatusParam> for ItemStatus {
    fn from(value: ItemStatusParam) -> Self {
        match value {
            ItemStatusParam::Active => Self::Active,
            ItemStatusParam::Completed => Self::Completed,
            ItemStatusParam::Archived => Self::Archived,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct ListItemsParams {
    group: String,
    kind: Option<ItemKindParam>,
    status: Option<ItemStatusParam>,
    updated_after: Option<i64>,
    limit: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct GetItemParams {
    id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct CreateGroupParams {
    group: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct RenameGroupParams {
    group: String,
    new_group: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct CreateItemParams {
    group: String,
    kind: ItemKindParam,
    title: String,
    #[serde(default)]
    content: String,
    #[serde(default)]
    tags: Vec<String>,
    due_at: Option<i64>,
    #[serde(default)]
    todo_entries: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct TodoEntryDraftParams {
    id: Option<String>,
    expected_revision: Option<i64>,
    text: String,
    #[serde(default)]
    is_completed: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct UpdateItemParams {
    id: String,
    expected_revision: i64,
    title: Option<String>,
    content: Option<String>,
    tags: Option<Vec<String>>,
    /// Set a timestamp to replace the due date. Omit it to preserve the current value.
    due_at: Option<i64>,
    /// Set true to remove the current due date. It cannot be combined with dueAt.
    #[serde(default)]
    clear_due_at: bool,
    /// Complete replacement checklist. Omit it to preserve all current TODO entries.
    replace_todo_entries: Option<Vec<TodoEntryDraftParams>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct SetStatusParams {
    id: String,
    expected_revision: i64,
    status: ItemStatusParam,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct MoveItemParams {
    id: String,
    expected_revision: i64,
    target_group: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct SetTodoEntryCompletedParams {
    item_id: String,
    entry_id: String,
    expected_item_revision: i64,
    expected_entry_revision: i64,
    is_completed: bool,
}

fn merge_update_input(
    current: McpItemView,
    params: UpdateItemParams,
) -> Result<McpUpdateItemInput, String> {
    if params.clear_due_at && params.due_at.is_some() {
        return Err("dueAt and clearDueAt cannot be used together".to_string());
    }
    if params.title.is_none()
        && params.content.is_none()
        && params.tags.is_none()
        && params.due_at.is_none()
        && !params.clear_due_at
        && params.replace_todo_entries.is_none()
    {
        return Err("at least one field must be supplied for update".to_string());
    }
    if current.revision != params.expected_revision {
        return Err("item changed elsewhere; reload before saving".to_string());
    }

    Ok(McpUpdateItemInput {
        id: params.id,
        expected_revision: params.expected_revision,
        title: params.title.unwrap_or(current.title),
        content: params.content.unwrap_or(current.content),
        tags: params.tags.unwrap_or(current.tags),
        due_at: if params.clear_due_at {
            None
        } else {
            params.due_at.or(current.due_at)
        },
        todo_entries: params.replace_todo_entries.map(|entries| {
            entries
                .into_iter()
                .map(|entry| McpTodoEntryDraftInput {
                    id: entry.id,
                    expected_revision: entry.expected_revision,
                    text: entry.text,
                    is_completed: entry.is_completed,
                })
                .collect()
        }),
    })
}

fn tool_result<T: Serialize>(result: Result<T, String>) -> CallToolResult {
    match result.and_then(|value| {
        serde_json::to_value(value).map_err(|error| format!("serialize MCP result failed: {error}"))
    }) {
        Ok(value) => CallToolResult::structured(value),
        Err(message) => CallToolResult::structured_error(serde_json::json!({ "error": message })),
    }
}

#[tool_router]
impl TipsMcpServer {
    #[tool(
        name = "tips_groups_list",
        description = "List task groups this authenticated MCP client can access."
    )]
    async fn groups_list(
        &self,
        Extension(parts): Extension<Parts>,
    ) -> Result<CallToolResult, ErrorData> {
        let principal = Self::principal(&parts)?;
        let client_id = principal.client_id;
        let result = self
            .call_store(move |store| store.mcp_list_groups(&client_id))
            .await;
        Ok(tool_result(result))
    }

    #[tool(
        name = "tips_group_create",
        description = "Create a persistent empty task group. The caller must have the explicit group-management capability enabled by the user; it receives coordinator access to the new group."
    )]
    async fn group_create(
        &self,
        Parameters(params): Parameters<CreateGroupParams>,
        Extension(parts): Extension<Parts>,
    ) -> Result<CallToolResult, ErrorData> {
        let principal = Self::principal(&parts)?;
        let client_id = principal.client_id;
        let result = self
            .call_store(move |store| {
                store.mcp_create_group(
                    &client_id,
                    McpCreateGroupInput {
                        group: params.group,
                    },
                )
            })
            .await;
        if let Ok(group) = &result {
            self.notify_group_changed(None, group.clone());
        }
        Ok(tool_result(result))
    }

    #[tool(
        name = "tips_group_rename",
        description = "Rename a persistent group and migrate all of its items and MCP grants atomically. The caller needs the explicit group-management capability and must be coordinator for that group; inbox cannot be renamed."
    )]
    async fn group_rename(
        &self,
        Parameters(params): Parameters<RenameGroupParams>,
        Extension(parts): Extension<Parts>,
    ) -> Result<CallToolResult, ErrorData> {
        let principal = Self::principal(&parts)?;
        let previous_key = params.group.trim().to_lowercase();
        let client_id = principal.client_id;
        let result = self
            .call_store(move |store| {
                store.mcp_rename_group(
                    &client_id,
                    McpRenameGroupInput {
                        group: params.group,
                        new_group: params.new_group,
                    },
                )
            })
            .await;
        if let Ok(group) = &result {
            self.notify_group_changed(Some(previous_key), group.clone());
        }
        Ok(tool_result(result))
    }

    #[tool(
        name = "tips_items_list",
        description = "List non-secret notes or TODO items in an authorized group. Desktop window state is never returned."
    )]
    async fn items_list(
        &self,
        Parameters(params): Parameters<ListItemsParams>,
        Extension(parts): Extension<Parts>,
    ) -> Result<CallToolResult, ErrorData> {
        let principal = Self::principal(&parts)?;
        let input = McpListItemsInput {
            group: params.group,
            kind: params.kind.map(Into::into),
            status: params.status.map(Into::into),
            updated_after: params.updated_after,
            limit: params.limit,
        };
        let client_id = principal.client_id;
        let result = self
            .call_store(move |store| store.mcp_list_items(&client_id, input))
            .await;
        Ok(tool_result(result))
    }

    #[tool(
        name = "tips_item_get",
        description = "Get one authorized non-secret note or TODO item by id."
    )]
    async fn item_get(
        &self,
        Parameters(params): Parameters<GetItemParams>,
        Extension(parts): Extension<Parts>,
    ) -> Result<CallToolResult, ErrorData> {
        let principal = Self::principal(&parts)?;
        let client_id = principal.client_id;
        let result = self
            .call_store(move |store| store.mcp_get_item(&client_id, &params.id))
            .await;
        Ok(tool_result(result))
    }

    #[tool(
        name = "tips_item_create",
        description = "Create a note or TODO item in a group where this client has write access. Secret items cannot be created through MCP."
    )]
    async fn item_create(
        &self,
        Parameters(params): Parameters<CreateItemParams>,
        Extension(parts): Extension<Parts>,
    ) -> Result<CallToolResult, ErrorData> {
        let principal = Self::principal(&parts)?;
        let input = McpCreateItemInput {
            group: params.group,
            kind: params.kind.into(),
            title: params.title,
            content: params.content,
            tags: params.tags,
            due_at: params.due_at,
            todo_entries: params.todo_entries,
        };
        let client_id = principal.client_id;
        let result: Result<McpItemView, String> = self
            .call_store(move |store| store.mcp_create_item(&client_id, input))
            .await;
        if let Ok(item) = &result {
            self.notify_items_changed(&item.id);
        }
        Ok(tool_result(result))
    }

    #[tool(
        name = "tips_item_update",
        description = "Patch note or TODO fields using an optimistic revision check. Omitted fields are preserved; clearDueAt removes a due date; replaceTodoEntries, when supplied, is the complete replacement checklist."
    )]
    async fn item_update(
        &self,
        Parameters(params): Parameters<UpdateItemParams>,
        Extension(parts): Extension<Parts>,
    ) -> Result<CallToolResult, ErrorData> {
        let principal = Self::principal(&parts)?;
        let item_id = params.id.clone();
        let lookup_client_id = principal.client_id.clone();
        let current = match self
            .call_store(move |store| store.mcp_get_item(&lookup_client_id, &item_id))
            .await
        {
            Ok(item) => item,
            Err(error) => return Ok(tool_result::<McpItemView>(Err(error))),
        };
        let input = match merge_update_input(current, params) {
            Ok(input) => input,
            Err(error) => return Ok(tool_result::<McpItemView>(Err(error))),
        };
        let client_id = principal.client_id;
        let result: Result<McpItemView, String> = self
            .call_store(move |store| store.mcp_update_item(&client_id, input))
            .await;
        if let Ok(item) = &result {
            self.notify_items_changed(&item.id);
        }
        Ok(tool_result(result))
    }

    #[tool(
        name = "tips_item_move",
        description = "Move a note or TODO to an existing group using an optimistic revision check. The caller must have write access to both source and target groups."
    )]
    async fn item_move(
        &self,
        Parameters(params): Parameters<MoveItemParams>,
        Extension(parts): Extension<Parts>,
    ) -> Result<CallToolResult, ErrorData> {
        let principal = Self::principal(&parts)?;
        let input = McpMoveItemInput {
            id: params.id,
            expected_revision: params.expected_revision,
            target_group: params.target_group,
        };
        let client_id = principal.client_id;
        let result = self
            .call_store(move |store| store.mcp_move_item(&client_id, input))
            .await;
        if let Ok(item) = &result {
            self.notify_items_changed(&item.id);
        }
        Ok(tool_result(result))
    }

    #[tool(
        name = "tips_item_set_status",
        description = "Set an item's active, completed, or archived status using an optimistic revision check."
    )]
    async fn item_set_status(
        &self,
        Parameters(params): Parameters<SetStatusParams>,
        Extension(parts): Extension<Parts>,
    ) -> Result<CallToolResult, ErrorData> {
        let principal = Self::principal(&parts)?;
        let input = McpSetStatusInput {
            id: params.id,
            expected_revision: params.expected_revision,
            status: params.status.into(),
        };
        let client_id = principal.client_id;
        let result: Result<McpItemView, String> = self
            .call_store(move |store| store.mcp_set_status(&client_id, input))
            .await;
        if let Ok(item) = &result {
            if item.status == ItemStatus::Archived {
                let store = self.app.state::<Store>();
                if let Err(error) = crate::windowing::hide_sticky(&self.app, &store, &item.id) {
                    eprintln!("hide sticky window after MCP archive failed: {error}");
                }
            }
            self.notify_items_changed(&item.id);
        }
        Ok(tool_result(result))
    }

    #[tool(
        name = "tips_todo_entry_set_completed",
        description = "Complete or reopen one TODO checklist entry using item and entry revision checks."
    )]
    async fn todo_entry_set_completed(
        &self,
        Parameters(params): Parameters<SetTodoEntryCompletedParams>,
        Extension(parts): Extension<Parts>,
    ) -> Result<CallToolResult, ErrorData> {
        let principal = Self::principal(&parts)?;
        let input = McpSetTodoEntryCompletedInput {
            item_id: params.item_id,
            entry_id: params.entry_id,
            expected_item_revision: params.expected_item_revision,
            expected_entry_revision: params.expected_entry_revision,
            is_completed: params.is_completed,
        };
        let client_id = principal.client_id;
        let result: Result<McpItemView, String> = self
            .call_store(move |store| store.mcp_set_todo_entry_completed(&client_id, input))
            .await;
        if let Ok(item) = &result {
            self.notify_items_changed(&item.id);
        }
        Ok(tool_result(result))
    }
}

#[tool_handler(
    name = "tips-desktop",
    version = "0.1.0",
    instructions = "Read and manage authorized Tips notes and TODO lists. Window position, appearance, and secret notes are outside this MCP interface."
)]
impl rmcp::ServerHandler for TipsMcpServer {}

#[cfg(test)]
mod tests {
    use super::{UpdateItemParams, merge_update_input, parse_bearer};
    use crate::{
        mcp_model::McpItemView,
        model::{ItemKind, ItemStatus},
    };

    #[test]
    fn bearer_parser_is_strict_about_shape_but_not_scheme_case() {
        assert_eq!(parse_bearer("Bearer abc"), Some("abc"));
        assert_eq!(parse_bearer("bearer abc"), Some("abc"));
        assert_eq!(parse_bearer(" Basic abc"), None);
        assert_eq!(parse_bearer("Bearer"), None);
        assert_eq!(parse_bearer("Bearer a b"), None);
        assert_eq!(parse_bearer("Bearer   "), None);
    }

    #[test]
    fn update_patch_preserves_omitted_fields_and_requires_current_revision() {
        let current = McpItemView {
            id: "item-1".to_string(),
            kind: ItemKind::Note,
            group: "inbox".to_string(),
            title: "Old title".to_string(),
            content: "Keep content".to_string(),
            status: ItemStatus::Active,
            tags: vec!["keep".to_string()],
            due_at: Some(1_900_000_000_000),
            completed_at: None,
            revision: 7,
            created_at: 1,
            updated_at: 2,
            archived_at: None,
            todo_entries: vec![],
        };
        let patch = UpdateItemParams {
            id: current.id.clone(),
            expected_revision: current.revision,
            title: Some("New title".to_string()),
            content: None,
            tags: None,
            due_at: None,
            clear_due_at: false,
            replace_todo_entries: None,
        };
        let merged = merge_update_input(current.clone(), patch).unwrap();
        assert_eq!(merged.title, "New title");
        assert_eq!(merged.content, "Keep content");
        assert_eq!(merged.tags, vec!["keep"]);
        assert_eq!(merged.due_at, Some(1_900_000_000_000));

        let clear_due_date = UpdateItemParams {
            id: current.id.clone(),
            expected_revision: current.revision,
            title: None,
            content: None,
            tags: None,
            due_at: None,
            clear_due_at: true,
            replace_todo_entries: None,
        };
        assert_eq!(
            merge_update_input(current.clone(), clear_due_date)
                .unwrap()
                .due_at,
            None
        );

        let stale = UpdateItemParams {
            id: current.id.clone(),
            expected_revision: current.revision - 1,
            title: Some("stale".to_string()),
            content: None,
            tags: None,
            due_at: None,
            clear_due_at: false,
            replace_todo_entries: None,
        };
        assert!(merge_update_input(current, stale).is_err());
    }
}
