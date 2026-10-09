use std::collections::HashMap;
use std::sync::Mutex;

use agent_doctor_core::{PromptSessionCancel, PromptSessionControl};

pub struct ActivePromptRun {
    pub cancel: PromptSessionCancel,
    pub control: PromptSessionControl,
    /// Webview that started this run. Events go only there, and only that
    /// window may cancel it.
    pub owner: String,
}

/// Several ask runs can proceed at once. Each chat session has its own slot,
/// so one project can keep working while another chat is sent.
pub struct PromptSessionState {
    runs: Mutex<HashMap<String, ActivePromptRun>>,
    /// Backend prompt-session id → client run key (chat session id, or window label).
    by_backend: Mutex<HashMap<String, String>>,
}

impl Default for PromptSessionState {
    fn default() -> Self {
        Self {
            runs: Mutex::new(HashMap::new()),
            by_backend: Mutex::new(HashMap::new()),
        }
    }
}

impl PromptSessionState {
    pub fn try_insert(&self, key: String, run: ActivePromptRun) -> Result<(), String> {
        let mut runs = self.runs.lock().map_err(|e| e.to_string())?;
        if runs.contains_key(&key) {
            return Err("another ask session is already running".into());
        }
        runs.insert(key, run);
        Ok(())
    }

    pub fn release(&self, key: &str) {
        if let Ok(mut runs) = self.runs.lock() {
            runs.remove(key);
        }
        if let Ok(mut index) = self.by_backend.lock() {
            index.retain(|_, client| client != key);
        }
    }

    pub fn note_backend(&self, backend_id: &str, client_key: &str) {
        if backend_id.is_empty() || client_key.is_empty() {
            return;
        }
        if let Ok(mut index) = self.by_backend.lock() {
            index.insert(backend_id.to_string(), client_key.to_string());
        }
    }

    pub fn control_for_backend(
        &self,
        backend_id: &str,
    ) -> Result<(PromptSessionControl, String), String> {
        let client = {
            let index = self.by_backend.lock().map_err(|e| e.to_string())?;
            index
                .get(backend_id)
                .cloned()
                .ok_or_else(|| "no active ask session for permission reply".to_string())?
        };
        let runs = self.runs.lock().map_err(|e| e.to_string())?;
        let run = runs
            .get(&client)
            .ok_or_else(|| "no active ask session for permission reply".to_string())?;
        Ok((run.control.clone(), run.owner.clone()))
    }

    /// Cancel one run. `client_run_id` empty cancels the legacy window-scoped slot.
    pub fn cancel_owned(&self, owner: &str, client_run_id: Option<String>) -> Result<bool, String> {
        let key = client_run_id
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
            .unwrap_or_else(|| owner.to_string());
        let runs = self.runs.lock().map_err(|e| e.to_string())?;
        let Some(run) = runs.get(&key) else {
            return Ok(false);
        };
        if run.owner != owner {
            return Ok(false);
        }
        run.cancel.request();
        Ok(true)
    }
}
