mod mcp_model;
mod mcp_server;
mod model;
mod native_i18n;
mod store;
mod sync_bundle;
mod windowing;

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::PathBuf,
};

use mcp_model::{
    CreateMcpClientInput, McpClientView, McpGroupGrantView, McpGroupRole, McpTransport,
    SetMcpGroupGrantInput,
};
use mcp_server::{McpRuntimeState, McpRuntimeStatus};
use model::{
    AddTodoEntryInput, CreateGroupInput, CreateItemInput, DeleteTodoEntryInput, GroupChangedEvent,
    Item, ItemGroup, LocalePreference, RenameGroupInput, SetStatusInput,
    SetTodoEntryCompletedInput, ThemeMode, UpdateItemInput, UpdateTodoEntryInput,
    WindowOpacityInput, WindowState,
};
#[cfg(debug_assertions)]
use model::{ItemColor, ItemKind, WindowGeometryInput};
use serde::Serialize;
use store::{Store, load_or_create_master_key};
use sync_bundle::{MAX_SYNC_BUNDLE_BYTES, SyncBundle, SyncExportSummary, SyncImportSummary};
use tauri::{
    AppHandle, Emitter, Manager, State, WindowEvent,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent, TrayIconId},
};
use tauri_plugin_dialog::DialogExt;

const TRAY_ID: &str = "main-tray";

struct NativeUi {
    show: MenuItem<tauri::Wry>,
    quit: MenuItem<tauri::Wry>,
    tray_id: TrayIconId,
}

#[tauri::command]
fn list_items(
    state: State<'_, Store>,
    include_archived: Option<bool>,
) -> Result<Vec<Item>, String> {
    state.list_items(include_archived.unwrap_or(false))
}

#[tauri::command]
fn get_item(state: State<'_, Store>, id: String) -> Result<Item, String> {
    state.get_item(&id)
}

#[tauri::command]
fn list_groups(state: State<'_, Store>) -> Result<Vec<ItemGroup>, String> {
    state.list_groups()
}

#[tauri::command]
fn create_group(
    _main_window: MainWindowOnly,
    app: AppHandle,
    state: State<'_, Store>,
    input: CreateGroupInput,
) -> Result<ItemGroup, String> {
    let group = state.create_group(input)?;
    app.emit(
        "groups-changed",
        GroupChangedEvent {
            previous_key: None,
            group: group.clone(),
        },
    )
    .map_err(|error| format!("notify group creation failed: {error}"))?;
    Ok(group)
}

#[tauri::command]
fn rename_group(
    _main_window: MainWindowOnly,
    app: AppHandle,
    state: State<'_, Store>,
    input: RenameGroupInput,
) -> Result<ItemGroup, String> {
    let previous_key = input.group.trim().to_lowercase();
    let group = state.rename_group(input)?;
    app.emit(
        "groups-changed",
        GroupChangedEvent {
            previous_key: Some(previous_key),
            group: group.clone(),
        },
    )
    .map_err(|error| format!("notify group rename failed: {error}"))?;
    Ok(group)
}

#[tauri::command]
fn create_item(
    app: AppHandle,
    state: State<'_, Store>,
    input: CreateItemInput,
) -> Result<Item, String> {
    let item = state.create_item(input)?;
    app.emit("items-changed", &item)
        .map_err(|error| format!("notify item creation failed: {error}"))?;
    Ok(item)
}

#[tauri::command]
fn update_item(
    app: AppHandle,
    state: State<'_, Store>,
    input: UpdateItemInput,
) -> Result<Item, String> {
    let item = state.update_item(input)?;
    app.emit("items-changed", &item)
        .map_err(|error| format!("notify item update failed: {error}"))?;
    Ok(item)
}

#[tauri::command]
fn set_item_status(
    app: AppHandle,
    state: State<'_, Store>,
    input: SetStatusInput,
) -> Result<Item, String> {
    let mut item = state.set_status(input)?;
    if item.status == model::ItemStatus::Archived {
        if let Err(error) = windowing::hide_sticky(&app, &state, &item.id) {
            eprintln!("hide sticky window after archive failed: {error}");
        } else {
            item = state.get_item(&item.id)?;
        }
    }
    app.emit("items-changed", &item)
        .map_err(|error| format!("notify status update failed: {error}"))?;
    Ok(item)
}

#[tauri::command]
fn add_todo_entry(
    app: AppHandle,
    state: State<'_, Store>,
    input: AddTodoEntryInput,
) -> Result<Item, String> {
    let item = state.add_todo_entry(input)?;
    app.emit("items-changed", &item)
        .map_err(|error| format!("notify TODO entry creation failed: {error}"))?;
    Ok(item)
}

#[tauri::command]
fn update_todo_entry(
    app: AppHandle,
    state: State<'_, Store>,
    input: UpdateTodoEntryInput,
) -> Result<Item, String> {
    let item = state.update_todo_entry(input)?;
    app.emit("items-changed", &item)
        .map_err(|error| format!("notify TODO entry update failed: {error}"))?;
    Ok(item)
}

#[tauri::command]
fn set_todo_entry_completed(
    app: AppHandle,
    state: State<'_, Store>,
    input: SetTodoEntryCompletedInput,
) -> Result<Item, String> {
    let item = state.set_todo_entry_completed(input)?;
    app.emit("items-changed", &item)
        .map_err(|error| format!("notify TODO entry status failed: {error}"))?;
    Ok(item)
}

#[tauri::command]
fn delete_todo_entry(
    app: AppHandle,
    state: State<'_, Store>,
    input: DeleteTodoEntryInput,
) -> Result<Item, String> {
    let item = state.delete_todo_entry(input)?;
    app.emit("items-changed", &item)
        .map_err(|error| format!("notify TODO entry deletion failed: {error}"))?;
    Ok(item)
}

#[tauri::command]
async fn open_sticky_window(app: AppHandle, item_id: String) -> Result<String, String> {
    // WebView2 IPC is delivered from a WebResourceRequested callback on Windows. Creating a
    // second webview synchronously inside that callback deadlocks controller initialization.
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<Store>();
        windowing::open_sticky(&app, &state, &item_id)
    })
    .await
    .map_err(|error| format!("open sticky window task failed: {error}"))?
}

#[tauri::command]
fn hide_sticky_window(
    app: AppHandle,
    state: State<'_, Store>,
    item_id: String,
) -> Result<(), String> {
    windowing::hide_sticky(&app, &state, &item_id)
}

#[tauri::command]
fn set_sticky_pin(
    app: AppHandle,
    state: State<'_, Store>,
    item_id: String,
    pinned: bool,
) -> Result<WindowState, String> {
    windowing::set_sticky_pin(&app, &state, &item_id, pinned)
}

#[tauri::command]
fn set_sticky_opacity(
    state: State<'_, Store>,
    input: WindowOpacityInput,
) -> Result<WindowState, String> {
    state.set_window_opacity(input)
}

#[tauri::command]
fn get_theme(state: State<'_, Store>) -> Result<ThemeMode, String> {
    state.get_theme()
}

#[tauri::command]
fn set_theme(
    app: AppHandle,
    state: State<'_, Store>,
    theme: ThemeMode,
) -> Result<ThemeMode, String> {
    let theme = state.set_theme(theme)?;
    app.emit("theme-changed", &theme)
        .map_err(|error| format!("notify theme update failed: {error}"))?;
    Ok(theme)
}

#[tauri::command]
fn get_locale(state: State<'_, Store>) -> Result<LocalePreference, String> {
    state.get_locale()
}

#[tauri::command]
fn set_locale(
    app: AppHandle,
    state: State<'_, Store>,
    locale: LocalePreference,
) -> Result<LocalePreference, String> {
    let locale = state.set_locale(locale)?;
    apply_native_locale(&app, locale);
    if let Err(error) = app.emit("locale-changed", &locale) {
        eprintln!("notify locale update failed after persistence: {error}");
    }
    Ok(locale)
}

#[tauri::command]
async fn export_sync_bundle(
    _main_window: MainWindowOnly,
    app: AppHandle,
    state: State<'_, Store>,
) -> Result<Option<SyncExportSummary>, String> {
    let Some(file_path) = app
        .dialog()
        .file()
        .add_filter("Tips progress", &["json"])
        .set_file_name("Tips-progress.json")
        .blocking_save_file()
    else {
        return Ok(None);
    };
    let mut path = file_path
        .into_path()
        .map_err(|error| format!("export requires a local file path: {error}"))?;
    if path.extension().is_none() {
        path.set_extension("json");
    }
    let (bundle, summary) = state.export_sync_bundle()?;
    let encoded = serde_json::to_vec_pretty(&bundle)
        .map_err(|error| format!("encode progress file failed: {error}"))?;
    if encoded.len() as u64 > MAX_SYNC_BUNDLE_BYTES {
        return Err(format!(
            "progress file is larger than the {} MiB limit",
            MAX_SYNC_BUNDLE_BYTES / 1024 / 1024
        ));
    }

    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&path)
        .map_err(|error| format!("open export file failed: {error}"))?;
    file.write_all(&encoded)
        .map_err(|error| format!("write progress file failed: {error}"))?;
    file.sync_all()
        .map_err(|error| format!("finish progress file failed: {error}"))?;
    Ok(Some(summary))
}

#[tauri::command]
async fn import_sync_bundle(
    _main_window: MainWindowOnly,
    app: AppHandle,
    state: State<'_, Store>,
) -> Result<Option<SyncImportSummary>, String> {
    let Some(file_path) = app
        .dialog()
        .file()
        .add_filter("Tips progress", &["json"])
        .blocking_pick_file()
    else {
        return Ok(None);
    };
    let path = file_path
        .into_path()
        .map_err(|error| format!("import requires a local file path: {error}"))?;
    let metadata = fs::metadata(&path)
        .map_err(|error| format!("read progress file metadata failed: {error}"))?;
    if !metadata.is_file() {
        return Err("the selected import path is not a file".to_string());
    }
    if metadata.len() > MAX_SYNC_BUNDLE_BYTES {
        return Err(format!(
            "progress file is larger than the {} MiB limit",
            MAX_SYNC_BUNDLE_BYTES / 1024 / 1024
        ));
    }

    let mut encoded = Vec::with_capacity(metadata.len() as usize);
    File::open(&path)
        .map_err(|error| format!("open progress file failed: {error}"))?
        .take(MAX_SYNC_BUNDLE_BYTES + 1)
        .read_to_end(&mut encoded)
        .map_err(|error| format!("read progress file failed: {error}"))?;
    if encoded.len() as u64 > MAX_SYNC_BUNDLE_BYTES {
        return Err(format!(
            "progress file is larger than the {} MiB limit",
            MAX_SYNC_BUNDLE_BYTES / 1024 / 1024
        ));
    }
    let bundle: SyncBundle = serde_json::from_slice(&encoded)
        .map_err(|error| format!("decode progress file failed: {error}"))?;
    let outcome = state.import_sync_bundle(bundle)?;

    for item_id in &outcome.changed_item_ids {
        let mut item = match state.get_item(item_id) {
            Ok(item) => item,
            Err(error) => {
                eprintln!("read imported item for UI notification failed: {error}");
                continue;
            }
        };
        if item.status == model::ItemStatus::Archived {
            if let Err(error) = windowing::hide_sticky(&app, &state, &item.id) {
                eprintln!("hide sticky window after import failed: {error}");
            } else if let Ok(refreshed) = state.get_item(&item.id) {
                item = refreshed;
            }
        }
        if let Err(error) = app.emit("items-changed", &item) {
            eprintln!("notify imported item update failed: {error}");
        }
    }
    Ok(Some(outcome.summary))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProvisionedMcpClient {
    client: McpClientView,
    bearer_token: String,
    grant: McpGroupGrantView,
    endpoint: String,
}

#[tauri::command]
fn get_mcp_runtime_status(state: State<'_, McpRuntimeState>) -> McpRuntimeStatus {
    state.snapshot()
}

struct MainWindowOnly;

impl<'de, R: tauri::Runtime> tauri::ipc::CommandArg<'de, R> for MainWindowOnly {
    fn from_command(
        command: tauri::ipc::CommandItem<'de, R>,
    ) -> Result<Self, tauri::ipc::InvokeError> {
        if command.message.webview_ref().label() != "main" {
            return Err("this operation is only available from the main window".into());
        }
        Ok(Self)
    }
}

#[tauri::command]
fn list_mcp_clients(
    _main_window: MainWindowOnly,
    state: State<'_, Store>,
) -> Result<Vec<McpClientView>, String> {
    state.list_mcp_clients(false)
}

#[tauri::command]
fn list_mcp_group_grants(
    _main_window: MainWindowOnly,
    state: State<'_, Store>,
) -> Result<Vec<McpGroupGrantView>, String> {
    state.list_mcp_group_grants(None, false)
}

#[tauri::command]
fn set_mcp_group_grant(
    _main_window: MainWindowOnly,
    state: State<'_, Store>,
    client_id: String,
    group_name: String,
    role: McpGroupRole,
) -> Result<McpGroupGrantView, String> {
    state.set_mcp_group_grant(SetMcpGroupGrantInput {
        client_id,
        group: group_name,
        role,
    })
}

#[tauri::command]
fn set_mcp_group_management(
    _main_window: MainWindowOnly,
    state: State<'_, Store>,
    client_id: String,
    allowed: bool,
) -> Result<(), String> {
    state.set_mcp_group_management(&client_id, allowed)
}

#[tauri::command]
fn provision_mcp_client(
    _main_window: MainWindowOnly,
    state: State<'_, Store>,
    runtime: State<'_, McpRuntimeState>,
    display_name: String,
    group_name: String,
    role: McpGroupRole,
    can_manage_groups: Option<bool>,
) -> Result<ProvisionedMcpClient, String> {
    let mut created = state.create_mcp_client(CreateMcpClientInput {
        display_name,
        transport: McpTransport::LoopbackHttp,
    })?;
    let grant = match state.set_mcp_group_grant(SetMcpGroupGrantInput {
        client_id: created.client.id.clone(),
        group: group_name,
        role,
    }) {
        Ok(grant) => grant,
        Err(error) => {
            let _ = state.revoke_mcp_client(&created.client.id);
            return Err(error);
        }
    };
    if can_manage_groups.unwrap_or(false) {
        if let Err(error) = state.set_mcp_group_management(&created.client.id, true) {
            let _ = state.revoke_mcp_group_grant(&created.client.id, &grant.group);
            let _ = state.revoke_mcp_client(&created.client.id);
            return Err(error);
        }
        created.client.can_manage_groups = true;
    }

    Ok(ProvisionedMcpClient {
        client: created.client,
        bearer_token: created.bearer_token,
        grant,
        endpoint: runtime.snapshot().endpoint,
    })
}

#[tauri::command]
fn revoke_mcp_client(
    _main_window: MainWindowOnly,
    state: State<'_, Store>,
    client_id: String,
) -> Result<(), String> {
    state.revoke_mcp_client(&client_id)
}

#[tauri::command]
fn revoke_mcp_group_grant(
    _main_window: MainWindowOnly,
    state: State<'_, Store>,
    client_id: String,
    group_name: String,
) -> Result<(), String> {
    state.revoke_mcp_group_grant(&client_id, &group_name)
}

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn build_tray(app: &mut tauri::App, locale: LocalePreference) -> tauri::Result<()> {
    let texts = native_i18n::texts_for_preference(locale);
    let show = MenuItem::with_id(app, "show", texts.tray_show, true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", texts.tray_quit, true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;

    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip(texts.tray_tooltip)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_main_window(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                show_main_window(tray.app_handle());
            }
        });

    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    let tray = builder.build(app)?;
    app.manage(NativeUi {
        show,
        quit,
        tray_id: tray.id().clone(),
    });
    Ok(())
}

fn apply_native_locale(app: &AppHandle, locale: LocalePreference) {
    let texts = native_i18n::texts_for_preference(locale);
    if let Some(window) = app.get_webview_window("main") {
        if let Err(error) = window.set_title(texts.main_title) {
            eprintln!("update main window title failed after locale persistence: {error}");
        }
    }
    if let Some(store) = app.try_state::<Store>() {
        if let Ok(items) = store.list_items(false) {
            for item in items.into_iter().filter(|item| item.title.is_empty()) {
                let label = windowing::sticky_label(&item.id);
                if let Some(window) = app.get_webview_window(&label) {
                    if let Err(error) = window.set_title(texts.sticky_fallback_title) {
                        eprintln!(
                            "update sticky window title failed after locale persistence: {error}"
                        );
                    }
                }
            }
        }
    }
    if let Some(native_ui) = app.try_state::<NativeUi>() {
        if let Err(error) = native_ui.show.set_text(texts.tray_show) {
            eprintln!("update tray show text failed after locale persistence: {error}");
        }
        if let Err(error) = native_ui.quit.set_text(texts.tray_quit) {
            eprintln!("update tray quit text failed after locale persistence: {error}");
        }
        if let Some(tray) = app.tray_by_id(&native_ui.tray_id) {
            if let Err(error) = tray.set_tooltip(Some(texts.tray_tooltip)) {
                eprintln!("update tray tooltip failed after locale persistence: {error}");
            }
        }
    }
}

fn resolve_data_dir(app: &tauri::App) -> Result<PathBuf, io::Error> {
    #[cfg(debug_assertions)]
    if let Ok(path) = std::env::var("TIPS_DATA_DIR") {
        if !path.trim().is_empty() {
            return Ok(PathBuf::from(path));
        }
    }

    app.path()
        .app_data_dir()
        .map_err(|error| io::Error::other(format!("resolve app data path failed: {error}")))
}

#[cfg(debug_assertions)]
fn run_debug_floating_window_smoke(app: &mut tauri::App) -> Result<(), io::Error> {
    if std::env::var("TIPS_SMOKE_FLOAT").as_deref() != Ok("1") {
        return Ok(());
    }

    let store = app.state::<Store>();
    store
        .set_theme(ThemeMode::BlackGold)
        .map_err(io::Error::other)?;
    let item = store
        .create_item(CreateItemInput {
            kind: ItemKind::Todo,
            group: Some("smoke/runtime".to_string()),
            title: "TODO List smoke".to_string(),
            content: "Checklist runtime verification".to_string(),
            color: Some(ItemColor::Sun),
            tags: vec!["smoke".to_string()],
            due_at: None,
            expires_at: None,
            mcp_access: None,
            todo_entries: vec![
                "确认窗口保持置顶".to_string(),
                "测试清单逐项勾选".to_string(),
                "保留背景与文字透明度".to_string(),
            ],
        })
        .map_err(io::Error::other)?;
    store
        .save_window_geometry(WindowGeometryInput {
            item_id: item.id.clone(),
            x: Some(96.0),
            y: Some(112.0),
            width: Some(360.0),
            height: Some(300.0),
            monitor_name: None,
            scale_factor: 1.0,
        })
        .map_err(io::Error::other)?;
    store
        .set_window_opacity(WindowOpacityInput {
            item_id: item.id.clone(),
            background_opacity: 0.65,
            text_opacity: 0.85,
        })
        .map_err(io::Error::other)?;
    let label = windowing::open_sticky(app.handle(), &store, &item.id).map_err(io::Error::other)?;
    eprintln!("TIPS_SMOKE_READY item_id={} window_label={label}", item.id);
    Ok(())
}

fn is_allowed_navigation(url: &tauri::Url, allow_dev_server: bool) -> bool {
    match (url.scheme(), url.host_str(), url.port()) {
        ("tauri", Some("localhost"), _) => true,
        ("http" | "https", Some("tauri.localhost"), None) => true,
        ("http", Some("localhost"), Some(1420)) if allow_dev_server => true,
        _ => false,
    }
}
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default().plugin(tauri_plugin_dialog::init());
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
        show_main_window(app);
    }));

    builder
        .plugin(
            tauri::plugin::Builder::<_, ()>::new("navigation-guard")
                .on_navigation(|_webview, url| is_allowed_navigation(url, cfg!(debug_assertions)))
                .build(),
        )
        .setup(|app| {
            let data_dir = resolve_data_dir(app)?;
            fs::create_dir_all(&data_dir)?;
            let key = load_or_create_master_key().map_err(io::Error::other)?;
            let store =
                Store::open(&data_dir.join("tips.sqlite3"), key).map_err(io::Error::other)?;
            app.manage(McpRuntimeState::default());
            app.manage(store);
            let locale = app
                .state::<Store>()
                .get_locale()
                .map_err(io::Error::other)?;
            build_tray(app, locale)?;
            apply_native_locale(app.handle(), locale);

            let mcp_config = app
                .state::<Store>()
                .get_mcp_server_config()
                .map_err(io::Error::other)?;
            mcp_server::spawn(app.handle().clone(), mcp_config);

            let state = app.state::<Store>();
            windowing::restore_visible_windows(app.handle(), &state);
            drop(state);
            #[cfg(debug_assertions)]
            run_debug_floating_window_smoke(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "main"
                && let WindowEvent::CloseRequested { api, .. } = event
            {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            list_items,
            get_item,
            list_groups,
            create_group,
            rename_group,
            create_item,
            update_item,
            set_item_status,
            add_todo_entry,
            update_todo_entry,
            set_todo_entry_completed,
            delete_todo_entry,
            open_sticky_window,
            hide_sticky_window,
            set_sticky_pin,
            set_sticky_opacity,
            get_theme,
            set_theme,
            get_locale,
            set_locale,
            export_sync_bundle,
            import_sync_bundle,
            get_mcp_runtime_status,
            list_mcp_clients,
            list_mcp_group_grants,
            set_mcp_group_grant,
            set_mcp_group_management,
            provision_mcp_client,
            revoke_mcp_client,
            revoke_mcp_group_grant,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Tips");
}

#[cfg(test)]
mod security_tests {
    use super::is_allowed_navigation;

    fn url(value: &str) -> tauri::Url {
        tauri::Url::parse(value).unwrap()
    }

    #[test]
    fn navigation_guard_allows_only_app_and_exact_dev_origins() {
        assert!(is_allowed_navigation(
            &url("tauri://localhost/index.html"),
            false
        ));
        assert!(is_allowed_navigation(
            &url("https://tauri.localhost/index.html?view=sticky"),
            false,
        ));
        assert!(is_allowed_navigation(&url("http://localhost:1420/"), true));

        assert!(!is_allowed_navigation(
            &url("http://localhost:1420/"),
            false
        ));
        assert!(!is_allowed_navigation(&url("http://localhost:1430/"), true));
        assert!(!is_allowed_navigation(&url("https://example.com/"), true));
        assert!(!is_allowed_navigation(&url("javascript:alert(1)"), true));
        assert!(!is_allowed_navigation(&url("data:text/html,unsafe"), true));
    }
}
