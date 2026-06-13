use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

#[tauri::command]
pub fn set_tray_text(app: AppHandle, text: String) -> Result<(), String> {
    let tray = app
        .tray_by_id("main")
        .ok_or_else(|| "tray not initialized".to_string())?;
    let value = if text.is_empty() {
        None
    } else {
        Some(text.as_str())
    };
    tray.set_title(value).map_err(|e| e.to_string())?;
    if !text.is_empty() {
        let _ = tray.set_tooltip(Some(&text));
    }
    Ok(())
}

#[tauri::command]
pub fn set_tray_menu(app: AppHandle, lines: Vec<String>) -> Result<(), String> {
    let mut info_items = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let item = MenuItemBuilder::with_id(format!("info-{i}"), line)
            .enabled(false)
            .build(&app)
            .map_err(|e| e.to_string())?;
        info_items.push(item);
    }
    let show = MenuItemBuilder::with_id("show", "Open CodeVetter")
        .build(&app)
        .map_err(|e| e.to_string())?;
    let quit = MenuItemBuilder::with_id("quit", "Quit")
        .build(&app)
        .map_err(|e| e.to_string())?;

    let mut builder = MenuBuilder::new(&app);
    for item in &info_items {
        builder = builder.item(item);
    }
    if !info_items.is_empty() {
        builder = builder.separator();
    }
    let menu = builder
        .item(&show)
        .item(&quit)
        .build()
        .map_err(|e| e.to_string())?;

    let tray = app
        .tray_by_id("main")
        .ok_or_else(|| "tray not initialized".to_string())?;
    tray.set_menu(Some(menu)).map_err(|e| e.to_string())?;
    Ok(())
}

/// Fire a system notification. Called from `useTrayMonitor` when an account's
/// worst-window utilization crosses a configured threshold.
#[tauri::command]
pub fn send_tray_notification(app: AppHandle, title: String, body: String) -> Result<(), String> {
    app.notification()
        .builder()
        .title(title)
        .body(body)
        .show()
        .map_err(|e| e.to_string())?;
    Ok(())
}
