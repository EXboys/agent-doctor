//! Keeps a TeamUps login alive after the browser covers this app.
//!
//! The page's timer stops being reliable once WeChat or the browser is in
//! front. This loop polls from the process, then brings the window that
//! started the login back.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use agent_doctor_core::{
    poll_teamups_login, sign_out_teamups, start_teamups_login, TeamupsLoginPollStatus,
    TeamupsLoginStart,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tauri_plugin_opener::OpenerExt;

use crate::windows::focus_named_window;

static NEXT_WATCH: AtomicU64 = AtomicU64::new(1);
static CURRENT_WATCH: AtomicU64 = AtomicU64::new(0);
static LAST_SITE_LOGIN_MS: AtomicU64 = AtomicU64::new(0);

/// `agent-doctor://login` from the account page.
fn is_site_login(url: &str) -> bool {
    let Some(rest) = url.trim().strip_prefix("agent-doctor:") else {
        return false;
    };
    let rest = rest.trim_start_matches('/');
    let head = rest.split(['/', '?', '#']).next().unwrap_or("");
    head.eq_ignore_ascii_case("login")
}

/// Website “打开 Agent Doctor”: show the app and start the same browser login
/// the in-app button uses. The site then approves because that person is
/// already signed in.
pub fn open_from_website(app: &AppHandle, urls: &[String]) {
    if !urls.iter().any(|url| is_site_login(url)) {
        return;
    }
    crate::windows::show_main_window(app);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let prev = LAST_SITE_LOGIN_MS.load(Ordering::SeqCst);
    if now.saturating_sub(prev) < 2500 {
        return;
    }
    LAST_SITE_LOGIN_MS.store(now, Ordering::SeqCst);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let started = tauri::async_runtime::spawn_blocking(start_teamups_login).await;
        let Ok(Ok(started)) = started else {
            return;
        };
        let url = started.verification_url.clone();
        watch(app.clone(), "main".to_string(), &started);
        let _ = app.opener().open_url(url, None::<&str>);
    });
}

#[derive(Clone, Serialize)]
struct TeamupsLoginFinished {
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

enum PollStep {
    Pending,
    Approved,
    Denied,
    Expired,
    Failed(String),
}

/// Drop the in-flight login. A poll that already saved a license is cleared
/// by the caller when this was a sign-out.
pub fn cancel() {
    CURRENT_WATCH.store(0, Ordering::SeqCst);
}

pub fn watch(app: AppHandle, window_label: String, start: &TeamupsLoginStart) {
    let ticket = NEXT_WATCH.fetch_add(1, Ordering::SeqCst);
    CURRENT_WATCH.store(ticket, Ordering::SeqCst);
    let device_code = start.device_code.clone();
    let expires = Duration::from_secs(start.expires_in_sec.max(30));
    tauri::async_runtime::spawn(async move {
        let started = Instant::now();
        let mut last_error = String::new();
        loop {
            if CURRENT_WATCH.load(Ordering::SeqCst) != ticket {
                return;
            }
            if started.elapsed() >= expires {
                finish(
                    &app,
                    "expired",
                    (!last_error.is_empty()).then_some(last_error),
                );
                return;
            }
            let code = device_code.clone();
            let polled =
                tauri::async_runtime::spawn_blocking(move || poll_teamups_login(&code)).await;
            let step = match polled {
                Ok(Ok(poll)) => match poll.status {
                    TeamupsLoginPollStatus::Pending => PollStep::Pending,
                    TeamupsLoginPollStatus::Approved => PollStep::Approved,
                    TeamupsLoginPollStatus::Denied => PollStep::Denied,
                    TeamupsLoginPollStatus::Expired => PollStep::Expired,
                },
                Ok(Err(error)) => PollStep::Failed(error.to_string()),
                Err(error) => PollStep::Failed(error.to_string()),
            };
            if matches!(step, PollStep::Approved) {
                // Sign-out wins over a poll that landed in the same moment.
                // A second login click does not: the scan the person finished
                // is the one that should sign the app in.
                if CURRENT_WATCH.load(Ordering::SeqCst) == 0 {
                    let _ = sign_out_teamups();
                    return;
                }
                CURRENT_WATCH.store(0, Ordering::SeqCst);
                focus_named_window(&app, &window_label);
                let _ = app.emit("teamups-account-changed", ());
                crate::channel_link::poke(&app);
                finish(&app, "approved", None);
                return;
            }
            if CURRENT_WATCH.load(Ordering::SeqCst) != ticket {
                return;
            }
            match step {
                PollStep::Pending => {}
                PollStep::Approved => return,
                PollStep::Denied => {
                    finish(&app, "denied", None);
                    return;
                }
                PollStep::Expired => {
                    finish(&app, "expired", None);
                    return;
                }
                PollStep::Failed(message) => last_error = message,
            }
            if started.elapsed() >= expires {
                let message = (!last_error.is_empty()).then_some(last_error);
                let status = if message.is_some() {
                    "failed"
                } else {
                    "expired"
                };
                finish(&app, status, message);
                return;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
}

fn finish(app: &AppHandle, status: &'static str, message: Option<String>) {
    let _ = app.emit(
        "teamups-login-finished",
        TeamupsLoginFinished { status, message },
    );
}

#[cfg(test)]
mod tests {
    use super::is_site_login;

    #[test]
    fn website_login_link_is_recognized() {
        assert!(is_site_login("agent-doctor://login"));
        assert!(is_site_login("agent-doctor://login/"));
        assert!(is_site_login("agent-doctor://login?from=account"));
        assert!(!is_site_login("agent-doctor://"));
        assert!(!is_site_login("https://teamups.vip/account"));
    }
}
