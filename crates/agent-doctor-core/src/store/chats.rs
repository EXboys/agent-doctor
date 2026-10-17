//! Chat projection on disk. The model keeps its own transcript; this file is what the window draws.
//!
//! One SQLite database, separate from `settings.db`, because a chat can contain text the user pasted.
//! Shape follows an event table: `sessions` holds the header, `events` holds each message in order.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use serde_json::{json, Value};

const SCHEMA_VERSION: &str = "1";
/// Tool output and collapsed logs stay bounded. User and assistant text are stored whole.
const TOOL_TEXT_CAP: usize = 16_000;

pub fn chats_db_path() -> Option<PathBuf> {
    super::db::agent_doctor_config_dir().map(|dir| dir.join("chats.db"))
}

pub struct ChatLog {
    conn: Connection,
}

impl ChatLog {
    pub fn open_default() -> Result<Self> {
        let path = chats_db_path().context("could not resolve agent-doctor config dir")?;
        Self::open_at(&path)
    }

    pub fn open_at(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)
            .with_context(|| format!("failed to open chat db at {}", path.display()))?;
        conn.busy_timeout(std::time::Duration::from_secs(2))?;
        conn.execute_batch(
            r#"
            PRAGMA journal_mode=WAL;
            CREATE TABLE IF NOT EXISTS meta (
                key TEXT PRIMARY KEY NOT NULL,
                value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY NOT NULL,
                position INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                header_json TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS events (
                session_id TEXT NOT NULL,
                seq INTEGER NOT NULL,
                event_json TEXT NOT NULL,
                PRIMARY KEY (session_id, seq)
            );
            "#,
        )?;
        conn.execute(
            "INSERT INTO meta (key, value) VALUES ('schema', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [SCHEMA_VERSION],
        )?;
        Ok(Self { conn })
    }

    /// `None` when nothing has been saved yet.
    pub fn load(&self) -> Result<Option<Value>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, header_json FROM sessions ORDER BY position ASC")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut sessions = Vec::new();
        for row in rows {
            let (id, header) = row?;
            let Ok(mut session) = serde_json::from_str::<Value>(&header) else {
                continue;
            };
            let Some(obj) = session.as_object_mut() else {
                continue;
            };
            obj.insert("id".into(), json!(id));
            obj.insert("messages".into(), self.events_for(&id)?);
            sessions.push(session);
        }
        if sessions.is_empty() {
            return Ok(None);
        }
        let active_id: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'active_id'",
                [],
                |row| row.get(0),
            )
            .ok();
        let active_id = active_id
            .filter(|id| {
                sessions
                    .iter()
                    .any(|session| session.get("id").and_then(|v| v.as_str()) == Some(id))
            })
            .unwrap_or_else(|| {
                sessions
                    .first()
                    .and_then(|session| session.get("id"))
                    .and_then(|id| id.as_str())
                    .unwrap_or("")
                    .to_string()
            });
        Ok(Some(json!({
            "activeId": active_id,
            "sessions": sessions,
        })))
    }

    /// Replace the saved projection. A failed write rolls back, so the previous commit stays.
    pub fn save(&self, store: &Value) -> Result<()> {
        let sessions = store
            .get("sessions")
            .and_then(|value| value.as_array())
            .context("chat store is missing sessions")?;
        let active_id = store
            .get("activeId")
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .to_string();
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM events", [])?;
        tx.execute("DELETE FROM sessions", [])?;
        for (position, session) in sessions.iter().enumerate() {
            let Some((id, updated_at, header, messages)) = split_session(session) else {
                continue;
            };
            tx.execute(
                "INSERT INTO sessions (id, position, updated_at, header_json) VALUES (?1, ?2, ?3, ?4)",
                params![id, position as i64, updated_at, header.to_string()],
            )?;
            for (seq, message) in messages.iter().enumerate() {
                let mut event = message.clone();
                cap_event(&mut event);
                tx.execute(
                    "INSERT INTO events (session_id, seq, event_json) VALUES (?1, ?2, ?3)",
                    params![id, seq as i64, event.to_string()],
                )?;
            }
        }
        tx.execute(
            "INSERT INTO meta (key, value) VALUES ('active_id', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [active_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Append or replace only the changed tail of the sessions included in the patch.
    /// Other sessions stay on disk. This is the same shape as an event log: new rows,
    /// or a rewrite from the first row that actually changed.
    pub fn apply_patch(&self, patch: &Value) -> Result<()> {
        let active_id = patch
            .get("activeId")
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .to_string();
        let order = string_list(patch.get("order"));
        let deleted = string_list(patch.get("deletedIds"));
        let sessions = patch
            .get("sessions")
            .and_then(|value| value.as_array())
            .map(|items| items.as_slice())
            .unwrap_or(&[]);
        let tx = self.conn.unchecked_transaction()?;
        for id in &deleted {
            tx.execute("DELETE FROM events WHERE session_id = ?1", [id])?;
            tx.execute("DELETE FROM sessions WHERE id = ?1", [id])?;
        }
        for session in sessions {
            let Some((id, updated_at, from_seq, header, messages)) = split_patch_session(session)
            else {
                continue;
            };
            let existing: i64 = tx.query_row(
                "SELECT COUNT(*) FROM events WHERE session_id = ?1",
                [&id],
                |row| row.get(0),
            )?;
            if from_seq > existing {
                anyhow::bail!("chat patch starts past the saved messages for {id}");
            }
            tx.execute(
                "INSERT INTO sessions (id, position, updated_at, header_json) VALUES (?1, 0, ?2, ?3)
                 ON CONFLICT(id) DO UPDATE SET updated_at = excluded.updated_at, header_json = excluded.header_json",
                params![id, updated_at, header.to_string()],
            )?;
            tx.execute(
                "DELETE FROM events WHERE session_id = ?1 AND seq >= ?2",
                params![id, from_seq],
            )?;
            for (offset, message) in messages.iter().enumerate() {
                let mut event = message.clone();
                cap_event(&mut event);
                tx.execute(
                    "INSERT INTO events (session_id, seq, event_json) VALUES (?1, ?2, ?3)",
                    params![id, from_seq + offset as i64, event.to_string()],
                )?;
            }
        }
        for (position, id) in order.iter().enumerate() {
            tx.execute(
                "UPDATE sessions SET position = ?1 WHERE id = ?2",
                params![position as i64, id],
            )?;
        }
        tx.execute(
            "INSERT INTO meta (key, value) VALUES ('active_id', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [active_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    fn events_for(&self, session_id: &str) -> Result<Value> {
        let mut stmt = self
            .conn
            .prepare("SELECT event_json FROM events WHERE session_id = ?1 ORDER BY seq ASC")?;
        let rows = stmt.query_map([session_id], |row| row.get::<_, String>(0))?;
        let mut messages = Vec::new();
        for row in rows {
            let raw = row?;
            if let Ok(message) = serde_json::from_str::<Value>(&raw) {
                messages.push(message);
            }
        }
        Ok(Value::Array(messages))
    }
}

fn string_list(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn split_patch_session(session: &Value) -> Option<(String, i64, i64, Value, Vec<Value>)> {
    let (id, updated_at, mut header, messages) = split_session(session)?;
    let from_seq = header.get("fromSeq").and_then(json_i64).unwrap_or(0).max(0);
    if let Some(obj) = header.as_object_mut() {
        obj.remove("fromSeq");
    }
    Some((id, updated_at, from_seq, header, messages))
}

fn split_session(session: &Value) -> Option<(String, i64, Value, Vec<Value>)> {
    let obj = session.as_object()?;
    let id = obj.get("id")?.as_str()?.trim();
    if id.is_empty() {
        return None;
    }
    let updated_at = obj.get("updatedAt").and_then(json_i64).unwrap_or(0);
    let mut header = obj.clone();
    let messages = header
        .remove("messages")
        .and_then(|value| match value {
            Value::Array(items) => Some(items),
            _ => None,
        })
        .unwrap_or_default();
    Some((id.to_string(), updated_at, Value::Object(header), messages))
}

fn json_i64(value: &Value) -> Option<i64> {
    value.as_i64().or_else(|| value.as_f64().map(|n| n as i64))
}

fn cap_event(event: &mut Value) {
    let Some(obj) = event.as_object_mut() else {
        return;
    };
    let role = obj
        .get("role")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let is_log = obj
        .get("log")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    if role == "tool" || (role == "meta" && is_log) {
        cap_field(obj, "content");
    }
    if let Some(step) = obj.get_mut("step").and_then(|value| value.as_object_mut()) {
        cap_field(step, "output");
    }
    if let Some(permission) = obj
        .get_mut("permission")
        .and_then(|value| value.as_object_mut())
    {
        cap_field(permission, "detail");
    }
}

fn cap_field(obj: &mut serde_json::Map<String, Value>, key: &str) {
    let Some(text) = obj.get(key).and_then(|value| value.as_str()) else {
        return;
    };
    if text.chars().count() <= TOOL_TEXT_CAP {
        return;
    }
    let trimmed: String = text.chars().take(TOOL_TEXT_CAP).collect();
    obj.insert(key.into(), json!(format!("{trimmed}\n…")));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db() -> (std::path::PathBuf, ChatLog) {
        let path = std::env::temp_dir().join(format!(
            "agent-doctor-chats-{}-{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = std::fs::remove_file(&path);
        let log = ChatLog::open_at(&path).expect("open");
        (path, log)
    }

    #[test]
    fn round_trip_keeps_header_fields_and_message_order() {
        let (path, log) = temp_db();
        let store = json!({
            "activeId": "b",
            "sessions": [{
                "id": "a",
                "title": "第一句",
                "runtime": "claude-code",
                "updatedAt": 10,
                "channelKey": "feishu-1",
                "messages": [
                    {"id": "m1", "role": "user", "content": "你好", "at": 1},
                    {"id": "m2", "role": "assistant", "content": "在", "at": 2}
                ]
            }, {
                "id": "b",
                "title": "第二句",
                "runtime": "codex",
                "updatedAt": 20,
                "messages": []
            }]
        });
        log.save(&store).expect("save");
        let loaded = log.load().expect("load").expect("some");
        assert_eq!(loaded["activeId"], "b");
        assert_eq!(loaded["sessions"][0]["channelKey"], "feishu-1");
        assert_eq!(loaded["sessions"][0]["messages"][1]["content"], "在");
        assert!(loaded["sessions"][0].get("messages").is_some());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn tool_text_is_capped_user_text_is_not() {
        let (path, log) = temp_db();
        let long = "x".repeat(TOOL_TEXT_CAP + 50);
        let store = json!({
            "activeId": "a",
            "sessions": [{
                "id": "a",
                "title": "",
                "runtime": "claude-code",
                "updatedAt": 1,
                "messages": [
                    {"id": "u", "role": "user", "content": long, "at": 1},
                    {"id": "t", "role": "tool", "content": long, "at": 2, "step": {"output": long}}
                ]
            }]
        });
        log.save(&store).unwrap();
        let loaded = log.load().unwrap().unwrap();
        let messages = &loaded["sessions"][0]["messages"];
        assert_eq!(messages[0]["content"].as_str().unwrap().len(), long.len());
        assert!(messages[1]["content"].as_str().unwrap().ends_with('…'));
        assert!(messages[1]["content"].as_str().unwrap().chars().count() < long.chars().count());
        assert!(messages[1]["step"]["output"]
            .as_str()
            .unwrap()
            .ends_with('…'));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn empty_database_loads_as_nothing() {
        let (path, log) = temp_db();
        assert!(log.load().unwrap().is_none());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn saving_again_drops_a_deleted_session() {
        let (path, log) = temp_db();
        log.save(&json!({
            "activeId": "a",
            "sessions": [
                {"id": "a", "updatedAt": 1, "messages": [{"id": "m", "role": "user", "content": "hi", "at": 1}]},
                {"id": "b", "updatedAt": 2, "messages": []}
            ]
        }))
        .unwrap();
        log.save(&json!({
            "activeId": "b",
            "sessions": [{"id": "b", "updatedAt": 3, "messages": []}]
        }))
        .unwrap();
        let loaded = log.load().unwrap().unwrap();
        assert_eq!(loaded["sessions"].as_array().unwrap().len(), 1);
        assert_eq!(loaded["sessions"][0]["id"], "b");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn patch_appends_one_message_and_leaves_the_other_session() {
        let (path, log) = temp_db();
        log.save(&json!({
            "activeId": "a",
            "sessions": [
                {"id": "a", "title": "旧", "updatedAt": 1, "messages": [
                    {"id": "m1", "role": "user", "content": "你好", "at": 1}
                ]},
                {"id": "b", "title": "旁边", "updatedAt": 2, "messages": [
                    {"id": "b1", "role": "user", "content": "别动我", "at": 1}
                ]}
            ]
        }))
        .unwrap();
        log.apply_patch(&json!({
            "v": 2,
            "activeId": "a",
            "order": ["a", "b"],
            "deletedIds": [],
            "sessions": [{
                "id": "a",
                "title": "旧",
                "updatedAt": 3,
                "fromSeq": 1,
                "messages": [{"id": "m2", "role": "assistant", "content": "在", "at": 2}]
            }]
        }))
        .unwrap();
        let loaded = log.load().unwrap().unwrap();
        assert_eq!(
            loaded["sessions"][0]["messages"].as_array().unwrap().len(),
            2
        );
        assert_eq!(loaded["sessions"][0]["messages"][0]["content"], "你好");
        assert_eq!(loaded["sessions"][0]["messages"][1]["content"], "在");
        assert_eq!(loaded["sessions"][1]["messages"][0]["content"], "别动我");
        assert!(loaded["sessions"][0].get("fromSeq").is_none());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn patch_replaces_the_tail_when_a_message_changes() {
        let (path, log) = temp_db();
        log.save(&json!({
            "activeId": "a",
            "sessions": [{"id": "a", "updatedAt": 1, "messages": [
                {"id": "m1", "role": "user", "content": "问", "at": 1},
                {"id": "m2", "role": "assistant", "content": "短", "at": 2}
            ]}]
        }))
        .unwrap();
        log.apply_patch(&json!({
            "v": 2,
            "activeId": "a",
            "order": ["a"],
            "deletedIds": [],
            "sessions": [{
                "id": "a",
                "updatedAt": 2,
                "fromSeq": 1,
                "messages": [{"id": "m2", "role": "assistant", "content": "更完整的回复", "at": 3}]
            }]
        }))
        .unwrap();
        let loaded = log.load().unwrap().unwrap();
        let messages = loaded["sessions"][0]["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1]["content"], "更完整的回复");
        let _ = std::fs::remove_file(&path);
    }
}
