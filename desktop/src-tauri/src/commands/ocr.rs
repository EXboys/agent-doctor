use agent_doctor_ocr::{read_image_texts, ImageTextReport};

/// Picture text must not run on the UI thread. On Windows the recognizer
/// completes through the calling thread's message pump; waiting there freezes
/// the window ("未响应") and the send never continues.
#[tauri::command]
pub async fn read_image_texts_command(paths: Vec<String>) -> Result<ImageTextReport, String> {
    tauri::async_runtime::spawn_blocking(move || read_image_texts(paths))
        .await
        .map_err(|err| err.to_string())
}
