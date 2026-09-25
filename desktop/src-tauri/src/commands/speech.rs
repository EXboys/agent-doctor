use crate::speech::{self, SpeechCapabilityDto, SpeechResultDto};
use agent_doctor_voice::{HostedInput, HostedState, HostedStep};
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

/// Hosted mode: emit `speech-event` final for each utterance until stopped.
#[tauri::command]
pub async fn voice_listen_start_command(
    app: AppHandle,
    language: Option<String>,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || speech::listen(app, language))
        .await
        .map_err(|e| format!("speech.failed:join error: {e}"))?
        .map_err(|e| e.to_command_error())
}

#[tauri::command]
pub fn voice_listen_stop_command() {
    speech::cancel_listen();
}

#[tauri::command]
pub async fn voice_speak_command(text: String, language: Option<String>) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || speech::speak(text, language))
        .await
        .map_err(|e| format!("speech.failed:join error: {e}"))?
        .map_err(|e| e.to_command_error())
}

#[tauri::command]
pub fn voice_speak_stop_command() {
    speech::cancel_speak();
}

#[tauri::command]
pub fn voice_hosted_reduce_command(state: HostedState, input: HostedInput) -> HostedStep {
    agent_doctor_voice::reduce(state, input)
}
