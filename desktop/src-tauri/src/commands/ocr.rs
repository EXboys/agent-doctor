use agent_doctor_ocr::{read_image_texts, ImageTextReport};
use agent_doctor_prompt::{ask_image_support_for_session, AskImageSupport};

/// Reads the saved provider (or this chat's pinned provider/model), off the UI thread.
#[tauri::command]
pub async fn ask_image_support_command(
    runtime: String,
    provider_id: Option<String>,
    model: Option<String>,
    workspace_name: Option<String>,
) -> Result<AskImageSupport, String> {
    tauri::async_runtime::spawn_blocking(move || {
        ask_image_support_for_session(
            &runtime,
            provider_id.as_deref(),
            model.as_deref(),
            workspace_name.as_deref(),
        )
    })
    .await
    .map_err(|err| err.to_string())
}

/// Picture text must not run on the UI thread. On Windows the recognizer
/// completes through the calling thread's message pump; waiting there freezes
/// the window ("未响应") and the send never continues.
#[tauri::command]
pub async fn read_image_texts_command(paths: Vec<String>) -> Result<ImageTextReport, String> {
    tauri::async_runtime::spawn_blocking(move || read_image_texts(paths))
        .await
        .map_err(|err| err.to_string())
}
