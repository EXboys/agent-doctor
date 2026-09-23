use crate::speech::{self, SpeechCapabilityDto, SpeechResultDto};
use tauri::AppHandle;

#[tauri::command]
pub fn speech_capability_command() -> SpeechCapabilityDto {
    speech::capability()
}

/// One-shot dictation. Emits `speech-event` for partial/final/error while running.
#[tauri::command]
pub async fn speech_dictate_command(
    app: AppHandle,
    language: Option<String>,
) -> Result<SpeechResultDto, String> {
    tauri::async_runtime::spawn_blocking(move || speech::dictate(app, language))
        .await
        .map_err(|e| format!("speech.failed:join error: {e}"))?
        .map_err(|e| e.to_command_error())
}

#[tauri::command]
pub fn speech_cancel_dictation_command() {
    speech::cancel_dictation();
}
