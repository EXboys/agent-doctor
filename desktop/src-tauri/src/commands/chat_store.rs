use agent_doctor_core::store::ChatLog;

#[tauri::command]
pub async fn load_chat_store_command() -> Result<Option<String>, String> {
    // Opening SQLite and decoding every session is real IO. A sync command
    // runs on the UI thread and freezes the window while it returns — the
    // same trap `mcp_status_command` documents.
    tauri::async_runtime::spawn_blocking(|| {
        let log = ChatLog::open_default().map_err(|error| error.to_string())?;
        let store = log.load().map_err(|error| error.to_string())?;
        Ok(store.map(|value| value.to_string()))
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn save_chat_store_command(store: String) -> Result<(), String> {
    // Same as loading: the patch lands in SQLite with a WAL fsync, which is
    // exactly the kind of write Defender makes slow on Windows.
    tauri::async_runtime::spawn_blocking(move || {
        let value: serde_json::Value =
            serde_json::from_str(&store).map_err(|error| error.to_string())?;
        let log = ChatLog::open_default().map_err(|error| error.to_string())?;
        let write = if value.get("v").and_then(|version| version.as_i64()) == Some(2) {
            log.apply_patch(&value)
        } else {
            log.save(&value)
        };
        write.map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

/// TEMP: dump the current chat store + the live transcript DOM so the fold
/// can be replayed locally without the app window.
#[tauri::command]
pub async fn ad_debug_dump_command(log_html: String) -> Result<String, String> {
    let log = ChatLog::open_default().map_err(|e| e.to_string())?;
    let store = log.load().map_err(|e| e.to_string())?;
    let data = serde_json::json!({
        "store": store,
        "log_html": log_html,
    });
    std::fs::write(
        "/tmp/ad-session-dump.json",
        serde_json::to_string_pretty(&data).unwrap(),
    )
    .map_err(|e| e.to_string())?;
    Ok("/tmp/ad-session-dump.json".to_string())
}
