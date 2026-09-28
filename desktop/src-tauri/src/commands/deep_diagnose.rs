use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use agent_doctor_core::{
    deep_diagnose_llm_config, run_deep_diagnose_chat, run_deep_repair, DeepDiagnoseEvent,
    DeepDiagnoseOptions, DeepDiagnoseReport, DeepDiagnoseTurn, DeepRepairSummary,
};
use tauri::{Emitter, State};

/// One deep-diagnose turn at a time; independent of the Ask prompt session.
#[derive(Default)]
pub struct DeepDiagnoseState {
    cancel: Mutex<Option<Arc<AtomicBool>>>,
}

#[tauri::command]
pub async fn deep_diagnose_chat_command(
    window: tauri::Window,
    state: State<'_, DeepDiagnoseState>,
    runtime: String,
    question: String,
    history: Option<Vec<DeepDiagnoseTurn>>,
    locale: Option<String>,
) -> Result<DeepDiagnoseReport, String> {
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut guard = state.cancel.lock().map_err(|e| e.to_string())?;
        if guard.is_some() {
            return Err("deep diagnose is already running".into());
        }
        *guard = Some(cancel.clone());
    }

    let options = DeepDiagnoseOptions {
        runtime_id: runtime,
        question,
        history: history.unwrap_or_default(),
        locale,
    };
    let emit_window = window.clone();
    let run_cancel = cancel.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let config = deep_diagnose_llm_config()?;
        run_deep_diagnose_chat(
            &options,
            &config,
            &run_cancel,
            |event: DeepDiagnoseEvent| {
                let _ = emit_window.emit_to(emit_window.label(), "deep-diagnose-event", &event);
            },
        )
    })
    .await;

    if let Ok(mut guard) = state.cancel.lock() {
        *guard = None;
    }
    result
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub fn cancel_deep_diagnose_command(state: State<'_, DeepDiagnoseState>) -> Result<bool, String> {
    let guard = state.cancel.lock().map_err(|e| e.to_string())?;
    if let Some(cancel) = guard.as_ref() {
        cancel.store(true, Ordering::SeqCst);
        Ok(true)
    } else {
        Ok(false)
    }
}

#[tauri::command]
pub async fn deep_repair_command(runtime: String) -> Result<DeepRepairSummary, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let config = deep_diagnose_llm_config()?;
        run_deep_repair(&runtime, &config)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("{e:#}"))
}
