mod model;
mod store;
mod windowing;

use std::{fs, io, path::PathBuf};

use model::{
    AddTodoEntryInput, CreateItemInput, DeleteTodoEntryInput, Item, SetStatusInput,
    SetTodoEntryCompletedInput, ThemeMode, UpdateItemInput, UpdateTodoEntryInput,
    WindowOpacityInput, WindowState,
};
#[cfg(debug_assertions)]
use model::{ItemColor, ItemKind, WindowGeometryInput};
use store::{Store, load_or_create_master_key};
use tauri::{
    AppHandle, Emitter, Manager, State, WindowEvent,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

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
    let item = state.set_status(input)?;
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
fn open_sticky_window(
    app: AppHandle,
    state: State<'_, Store>,
    item_id: String,
) -> Result<String, String> {
    windowing::open_sticky(&app, &state, &item_id)
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

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn build_tray(app: &mut tauri::App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "显示主面板", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出 Tips", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;

    let mut builder = TrayIconBuilder::new()
        .tooltip("Tips · 本地便签")
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
    builder.build(app)?;
    Ok(())
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
    tauri::Builder::default()
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
            app.manage(store);
            build_tray(app)?;

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
