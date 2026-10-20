use std::collections::HashMap;
use std::sync::Mutex;

use agent_doctor_prompt::{PromptSessionCancel, PromptSessionControl, PromptSessionEvent};
use serde::Serialize;

pub struct ActivePromptRun {
    pub cancel: PromptSessionCancel,
    pub control: PromptSessionControl,
    /// Webview that started this run. Events go only there, and only that
    /// window may cancel it.
    pub owner: String,
    /// Questions still waiting for an answer. A reloaded page has lost its cards
    /// and the turn would wait on them forever.
    pub asks: Vec<PromptSessionEvent>,
}

impl ActivePromptRun {
    pub fn new(cancel: PromptSessionCancel, control: PromptSessionControl, owner: String) -> Self {
        Self {
            cancel,
            control,
            owner,
            asks: Vec::new(),
        }
    }
}

/// A run still going, as a reloaded page needs it to pick the run back up.
#[derive(Debug, Clone, Serialize)]
pub struct LivePromptRun {
    pub client_run_id: String,
    pub session_id: Option<String>,
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

    /// Keep unanswered questions so a reloaded page can show them again.
    pub fn note_event(&self, client_key: &str, event: &PromptSessionEvent) {
        let Ok(mut runs) = self.runs.lock() else {
            return;
        };
        let Some(run) = runs.get_mut(client_key) else {
            return;
        };
        match event {
            PromptSessionEvent::PermissionRequest { request_id, .. } => {
                run.asks
                    .retain(|ask| ask_id(ask) != Some(request_id.as_str()));
                run.asks.push(event.clone());
            }
            PromptSessionEvent::PermissionResolved { request_id, .. } => {
                run.asks
                    .retain(|ask| ask_id(ask) != Some(request_id.as_str()));
            }
            PromptSessionEvent::Completed { .. } => run.asks.clear(),
            _ => {}
        }
    }

    pub fn forget_ask(&self, backend_id: &str, request_id: &str) {
        let client = self
            .by_backend
            .lock()
            .ok()
            .and_then(|index| index.get(backend_id).cloned());
        let Some(client) = client else {
            return;
        };
        if let Ok(mut runs) = self.runs.lock() {
            if let Some(run) = runs.get_mut(&client) {
                run.asks.retain(|ask| ask_id(ask) != Some(request_id));
            }
        }
    }

    /// Runs this window started that are still going.
    pub fn live_runs(&self, owner: &str) -> Vec<LivePromptRun> {
        let backends: HashMap<String, String> = self
            .by_backend
            .lock()
            .map(|index| {
                index
                    .iter()
                    .map(|(backend, client)| (client.clone(), backend.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let Ok(runs) = self.runs.lock() else {
            return Vec::new();
        };
        runs.iter()
            .filter(|(_, run)| run.owner == owner)
            .map(|(client, _)| LivePromptRun {
                client_run_id: client.clone(),
                session_id: backends.get(client).cloned(),
            })
            .collect()
    }

    pub fn pending_asks(&self, owner: &str, client_key: &str) -> Vec<PromptSessionEvent> {
        let Ok(runs) = self.runs.lock() else {
            return Vec::new();
        };
        runs.get(client_key)
            .filter(|run| run.owner == owner)
            .map(|run| run.asks.clone())
            .unwrap_or_default()
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

fn ask_id(event: &PromptSessionEvent) -> Option<&str> {
    match event {
        PromptSessionEvent::PermissionRequest { request_id, .. } => Some(request_id.as_str()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask(id: &str) -> PromptSessionEvent {
        PromptSessionEvent::PermissionRequest {
            session_id: "ps-1".into(),
            request_id: id.into(),
            tool_name: "Bash".into(),
            detail: "ls".into(),
            input_json: "{}".into(),
            input_mode: "choice".into(),
        }
    }

    #[test]
    fn a_reloaded_window_finds_its_run_and_open_questions() {
        let state = PromptSessionState::default();
        state
            .try_insert(
                "chat-1".into(),
                ActivePromptRun::new(
                    PromptSessionCancel::new(),
                    PromptSessionControl::new(),
                    "ask".into(),
                ),
            )
            .unwrap();
        state.note_backend("ps-1", "chat-1");
        state.note_event("chat-1", &ask("r1"));
        state.note_event("chat-1", &ask("r2"));
        state.forget_ask("ps-1", "r1");

        let live = state.live_runs("ask");
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].client_run_id, "chat-1");
        assert_eq!(live[0].session_id.as_deref(), Some("ps-1"));
        assert!(state.live_runs("diagnose").is_empty());

        let asks = state.pending_asks("ask", "chat-1");
        assert_eq!(asks.len(), 1);
        assert_eq!(ask_id(&asks[0]), Some("r2"));

        state.release("chat-1");
        assert!(state.live_runs("ask").is_empty());
    }
}
