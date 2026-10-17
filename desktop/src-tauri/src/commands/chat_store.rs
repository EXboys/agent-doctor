use agent_doctor_core::store::ChatLog;

#[tauri::command]
pub fn load_chat_store_command() -> Result<Option<String>, String> {
    let log = ChatLog::open_default().map_err(|error| error.to_string())?;
    let store = log.load().map_err(|error| error.to_string())?;
    Ok(store.map(|value| value.to_string()))
}

#[tauri::command]
pub fn save_chat_store_command(store: String) -> Result<(), String> {
    let value: serde_json::Value =
        serde_json::from_str(&store).map_err(|error| error.to_string())?;
    let log = ChatLog::open_default().map_err(|error| error.to_string())?;
    let write = if value.get("v").and_then(|version| version.as_i64()) == Some(2) {
        log.apply_patch(&value)
    } else {
        log.save(&value)
    };
    write.map_err(|error| error.to_string())
}
