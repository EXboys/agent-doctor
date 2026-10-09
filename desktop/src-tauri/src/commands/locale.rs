use tauri::{AppHandle, Emitter, Manager};

fn locale_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    Ok(app
        .path()
        .app_config_dir()
        .map_err(|error| error.to_string())?
        .join("locale"))
}

#[tauri::command]
pub fn get_app_locale_command(app: AppHandle) -> Option<String> {
    let raw = std::fs::read_to_string(locale_path(&app).ok()?).ok()?;
    match raw.trim() {
        "en" | "zh" => Some(raw.trim().to_string()),
        _ => None,
    }
}

#[tauri::command]
pub fn set_app_locale_command(app: AppHandle, locale: String) -> Result<(), String> {
    if locale != "en" && locale != "zh" {
        return Err("unsupported locale".into());
    }
    let path = locale_path(&app)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    }
    std::fs::write(&path, &locale).map_err(|error| error.to_string())?;
    let _ = app.emit("locale-changed", locale);
    Ok(())
}
