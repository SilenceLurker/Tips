use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};

use crate::{
    model::{WindowGeometryInput, WindowState},
    store::Store,
};

pub fn sticky_label(item_id: &str) -> String {
    format!("sticky-{}", item_id.replace('-', ""))
}

pub fn open_sticky(app: &AppHandle, store: &Store, item_id: &str) -> Result<String, String> {
    let item = store.get_item(item_id)?;
    let state = item
        .window
        .clone()
        .ok_or_else(|| "item has no window state".to_string())?;
    let label = sticky_label(item_id);

    if let Some(window) = app.get_webview_window(&label) {
        window
            .set_always_on_top(state.always_on_top)
            .map_err(|error| format!("restore pin state failed: {error}"))?;
        window
            .show()
            .map_err(|error| format!("show sticky window failed: {error}"))?;
        window
            .set_focus()
            .map_err(|error| format!("focus sticky window failed: {error}"))?;
        store.set_window_visible(item_id, true)?;
        return Ok(label);
    }

    let url = WebviewUrl::App(format!("index.html?view=sticky&id={item_id}").into());
    let mut builder = WebviewWindowBuilder::new(app, &label, url)
        .title(if item.title.is_empty() {
            "Tips 便签"
        } else {
            &item.title
        })
        .inner_size(state.width, state.height)
        .min_inner_size(240.0, 180.0)
        .resizable(true)
        .decorations(false)
        .always_on_top(state.always_on_top)
        .skip_taskbar(true)
        .shadow(true)
        .transparent(true)
        .background_color(tauri::window::Color(0, 0, 0, 0));

    if let (Some(x), Some(y)) = (state.x, state.y) {
        builder = builder.position(x, y);
    }

    let window = builder
        .build()
        .map_err(|error| format!("create sticky window failed: {error}"))?;
    observe_window(app, &window, item_id.to_string());
    store.set_window_visible(item_id, true)?;
    Ok(label)
}

pub fn hide_sticky(app: &AppHandle, store: &Store, item_id: &str) -> Result<(), String> {
    let label = sticky_label(item_id);
    if let Some(window) = app.get_webview_window(&label) {
        window
            .hide()
            .map_err(|error| format!("hide sticky window failed: {error}"))?;
    }
    store.set_window_visible(item_id, false)
}

pub fn set_sticky_pin(
    app: &AppHandle,
    store: &Store,
    item_id: &str,
    pinned: bool,
) -> Result<WindowState, String> {
    let label = sticky_label(item_id);
    if let Some(window) = app.get_webview_window(&label) {
        window
            .set_always_on_top(pinned)
            .map_err(|error| format!("set window pin failed: {error}"))?;
    }
    store.set_window_pin(item_id, pinned)
}

pub fn restore_visible_windows(app: &AppHandle, store: &Store) {
    if let Ok(items) = store.list_items(false) {
        for item in items {
            if item.window.as_ref().is_some_and(|window| window.is_visible) {
                let _ = open_sticky(app, store, &item.id);
            }
        }
    }
}

fn observe_window(app: &AppHandle, window: &WebviewWindow, item_id: String) {
    let app_handle = app.clone();
    let observed_window = window.clone();
    window.on_window_event(move |event| {
        let store = app_handle.state::<Store>();
        let scale_factor = observed_window.scale_factor().unwrap_or(1.0).max(0.1);

        match event {
            WindowEvent::Moved(position) => {
                let _ = store.save_window_geometry(WindowGeometryInput {
                    item_id: item_id.clone(),
                    x: Some(position.x as f64 / scale_factor),
                    y: Some(position.y as f64 / scale_factor),
                    width: None,
                    height: None,
                    monitor_name: observed_window
                        .current_monitor()
                        .ok()
                        .flatten()
                        .and_then(|monitor| monitor.name().map(ToOwned::to_owned)),
                    scale_factor,
                });
            }
            WindowEvent::Resized(size) => {
                let _ = store.save_window_geometry(WindowGeometryInput {
                    item_id: item_id.clone(),
                    x: None,
                    y: None,
                    width: Some(size.width as f64 / scale_factor),
                    height: Some(size.height as f64 / scale_factor),
                    monitor_name: observed_window
                        .current_monitor()
                        .ok()
                        .flatten()
                        .and_then(|monitor| monitor.name().map(ToOwned::to_owned)),
                    scale_factor,
                });
            }
            WindowEvent::CloseRequested { .. } | WindowEvent::Destroyed => {
                let _ = store.set_window_visible(&item_id, false);
            }
            _ => {}
        }
    });
}
