//! One rolling log for the Ask prompt-session lifecycle, so an "允许没有送出"
//! report can be traced afterwards: asks arriving, each allow/deny send with
//! its error class and duration, and how each run ends.
//!
//! Never logged: prompt text, commands, file paths. The privacy line matches
//! the install ping — ids, tool names, classes, timings only. Best effort:
//! every failure here is silent; the chat must not depend on this file.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

const LOG_FILE: &str = "prompt-session.log";
/// The log restarts from empty once it holds roughly this much.
const MAX_BYTES: u64 = 1_000_000;
/// Field values are trimmed to this many characters.
const FIELD_MAX_CHARS: usize = 64;

/// `None` until startup points it at the app log dir. The same lock covers
/// reading the path and appending, so lines never interleave mid-write.
static LOG_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Point the log at `<dir>/prompt-session.log`. Called once at startup;
/// without it, logging stays off.
pub fn init(dir: PathBuf) {
    *log_state() = Some(dir.join(LOG_FILE));
}

/// The whole log text, when logging is on and the file exists. The manual
/// diagnosis report sends this; it holds ids and error classes only.
pub fn read_log() -> Option<String> {
    let path = log_state().as_ref()?.clone();
    fs::read_to_string(path).ok()
}

fn log_state() -> MutexGuard<'static, Option<PathBuf>> {
    LOG_PATH
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Append one event line: `<utc> <kind> key=value …`.
pub fn event(kind: &str, fields: &[(&str, &str)]) {
    let mut line = String::with_capacity(120);
    line.push_str(&utc_now());
    line.push(' ');
    line.push_str(kind);
    for (key, value) in fields {
        line.push(' ');
        line.push_str(key);
        line.push('=');
        line.push_str(&field_value(value));
    }
    line.push('\n');

    let state = log_state();
    let Some(path) = state.as_ref() else {
        return;
    };
    write_line(path, &line, MAX_BYTES);
}

fn field_value(value: &str) -> String {
    let flat = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= FIELD_MAX_CHARS {
        flat
    } else {
        flat.chars().take(FIELD_MAX_CHARS).collect()
    }
}

/// Append, restarting the file when it has grown past `max_bytes`. Any
/// failure is swallowed — a broken log must never break a reply.
fn write_line(path: &Path, line: &str, max_bytes: u64) {
    let grown = fs::metadata(path)
        .map(|meta| meta.len() >= max_bytes)
        .unwrap_or(false);
    if grown {
        let _ = fs::write(path, b"");
    }
    let attempt = || -> std::io::Result<()> {
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        file.write_all(line.as_bytes())
    };
    if attempt().is_err() {
        // The directory may be missing after a cleanup; recreate and try once.
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = attempt();
    }
}

/// UTC wall clock as `YYYY-MM-DDTHH:MM:SS.mmmZ`, no date library.
fn utc_now() -> String {
    let since_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = since_epoch.as_secs();
    let millis = since_epoch.subsec_millis();
    let (hour, minute, second) = ((secs / 3_600) % 24, (secs / 60) % 60, secs % 60);
    let (year, month, day) = civil_from_date(secs / 86_400);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
}

/// Days since the epoch to (year, month, day) — Howard Hinnant's algorithm.
fn civil_from_date(days: u64) -> (i64, u64, u64) {
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (
        if month <= 2 { year + 1 } else { year },
        month as u64,
        day as u64,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as TestMutex;

    /// The log path is process-wide, so tests that touch it serialize here.
    static TEST_LOCK: TestMutex<()> = TestMutex::new(());

    fn point_log(dir: Option<&Path>) {
        *log_state() = dir.map(|dir| dir.join(LOG_FILE));
    }

    fn unique_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("ad-prompt-log-test-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn events_append_one_line_each_and_unpointed_logging_is_silent() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        point_log(None);
        event("resolve_ok", &[("session", "abc")]); // no file, no panic

        let dir = unique_dir("append");
        point_log(Some(&dir));
        event("resolve_ok", &[("session", "abc"), ("ms", "12")]);
        event("resolve_err", &[("class", "dead-pipe")]);
        point_log(None);

        let content = fs::read_to_string(dir.join(LOG_FILE)).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(
            lines[0].ends_with("resolve_ok session=abc ms=12"),
            "{}",
            lines[0]
        );
        assert!(lines[1].contains("resolve_err class=dead-pipe"));
        assert!(lines[0].starts_with("20"), "utc stamp: {}", lines[0]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_log_returns_the_text_only_while_pointed() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        point_log(None);
        assert_eq!(read_log(), None);

        let dir = unique_dir("read");
        point_log(Some(&dir));
        event("resolve_err", &[("class", "dead-pipe")]);
        let text = read_log().expect("log text after an event");
        assert!(text.contains("resolve_err class=dead-pipe"), "{text}");
        point_log(None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn values_are_flattened_and_trimmed() {
        assert_eq!(field_value("a\nb  c"), "a b c");
        let long = "x".repeat(100);
        assert_eq!(field_value(&long).chars().count(), FIELD_MAX_CHARS);
    }

    #[test]
    fn file_restarts_once_it_grows_past_the_limit() {
        let dir = unique_dir("restart");
        let path = dir.join(LOG_FILE);
        write_line(&path, "first-very-long-line\n", 30);
        write_line(&path, "second-very-long-line\n", 30);
        write_line(&path, "third\n", 30);
        let content = fs::read_to_string(&path).unwrap();
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(content, "third\n", "log should have restarted: {content}");
    }

    #[test]
    fn civil_dates_match_known_days() {
        assert_eq!(civil_from_date(0), (1970, 1, 1));
        // 2026-11-09 is day 20766 after 1970-01-01.
        assert_eq!(civil_from_date(20_766), (2026, 11, 9));
    }
}
