//! One checklist for every agent version Agent Doctor hosts.
//!
//! Older Claude builds and Codex send a whole list each time (`TodoWrite`,
//! `update_plan` / `todo_write`, `turn/plan/updated`). Current Claude builds
//! add one task (`TaskCreate`) and then patch it by id (`TaskUpdate`).
//! Both land on [`PlanBoard`], and the chat window only ever sees the list.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

const MAX_STEPS: usize = 24;
const MAX_TEXT: usize = 180;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStepState {
    Pending,
    Doing,
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanStep {
    pub text: String,
    pub state: PlanStepState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Draft {
    id: String,
    text: String,
    doing: String,
    state: PlanStepState,
}

#[derive(Debug, Default)]
pub(crate) struct PlanBoard {
    steps: Vec<Draft>,
    /// tool_use id → normalized tool name, so a later tool result can finish the update.
    tools: HashMap<String, String>,
}

impl PlanBoard {
    /// A tool call that might change the checklist. `None` means this tool is not a plan.
    pub(crate) fn observe_tool(
        &mut self,
        name: &str,
        tool_use_id: &str,
        input: &Value,
    ) -> Option<Vec<PlanStep>> {
        let normalized = normalize_tool_name(name);
        if tool_carries_plan(name) && !tool_use_id.is_empty() {
            self.tools
                .insert(tool_use_id.to_string(), normalized.clone());
        }
        if let Some(drafts) = drafts_from_tool(name, input) {
            self.steps = drafts;
            self.tools.clear();
            return Some(self.display());
        }
        match normalized.as_str() {
            "taskcreate" => {
                let text = first_string(input, &["subject", "description", "content", "title"]);
                if text.is_empty() {
                    return None;
                }
                let doing = first_string(input, &["activeForm", "active_form"]);
                let id = if tool_use_id.is_empty() {
                    format!("create-{}", self.steps.len())
                } else {
                    tool_use_id.to_string()
                };
                if let Some(step) = self.steps.iter_mut().find(|step| step.id == id) {
                    step.text = text;
                    if !doing.is_empty() {
                        step.doing = doing;
                    }
                } else {
                    self.push(Draft {
                        id,
                        text,
                        doing,
                        state: PlanStepState::Pending,
                    });
                }
                Some(self.display())
            }
            "taskupdate" => self.apply_update(input),
            _ => None,
        }
    }

    /// Claude sends the assigned task id on the following user `tool_result`.
    pub(crate) fn observe_user(&mut self, value: &Value) -> Option<Vec<PlanStep>> {
        let result = value.get("tool_use_result");
        let blocks = value
            .pointer("/message/content")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut changed = false;
        for block in blocks {
            if block.get("type").and_then(Value::as_str) != Some("tool_result") {
                continue;
            }
            let Some(tool_use_id) = block.get("tool_use_id").and_then(Value::as_str) else {
                continue;
            };
            let tool = self.tools.get(tool_use_id).cloned().unwrap_or_default();
            if tool == "taskcreate" {
                if let Some(task_id) = task_id_from_result(result) {
                    if let Some(step) = self.steps.iter_mut().find(|step| step.id == tool_use_id) {
                        step.id = task_id;
                        changed = true;
                    }
                }
            } else if tool == "tasklist" {
                if let Some(drafts) = drafts_from_value(result.unwrap_or(&Value::Null)) {
                    self.steps = drafts;
                    changed = true;
                }
            }
        }
        if !changed {
            if let Some(task_id) = task_id_from_result(result) {
                let pending_ids: Vec<String> = self
                    .steps
                    .iter()
                    .filter(|step| {
                        self.tools
                            .get(&step.id)
                            .is_some_and(|name| name == "taskcreate")
                    })
                    .map(|step| step.id.clone())
                    .collect();
                if let [old_id] = pending_ids.as_slice() {
                    if let Some(step) = self.steps.iter_mut().find(|step| step.id == *old_id) {
                        step.id = task_id;
                        changed = true;
                    }
                }
            }
        }
        if changed {
            Some(self.display())
        } else {
            None
        }
    }

    fn apply_update(&mut self, input: &Value) -> Option<Vec<PlanStep>> {
        let id = first_string(input, &["taskId", "task_id", "id"]);
        if id.is_empty() {
            return None;
        }
        let status = first_string(input, &["status", "state"]);
        if normalize_tool_name(&status) == "deleted" {
            let before = self.steps.len();
            self.steps.retain(|step| step.id != id);
            return (self.steps.len() != before).then(|| self.display());
        }
        let text = first_string(input, &["subject", "content", "title", "description"]);
        let doing = first_string(input, &["activeForm", "active_form"]);
        if let Some(step) = self.steps.iter_mut().find(|step| step.id == id) {
            if !status.is_empty() {
                step.state = map_state(&status);
            }
            if !text.is_empty() {
                step.text = clip_chars(&text, MAX_TEXT);
            }
            if !doing.is_empty() {
                step.doing = clip_chars(&doing, MAX_TEXT);
            }
            return Some(self.display());
        }
        if text.is_empty() {
            return None;
        }
        self.push(Draft {
            id,
            text,
            doing,
            state: if status.is_empty() {
                PlanStepState::Pending
            } else {
                map_state(&status)
            },
        });
        Some(self.display())
    }

    fn push(&mut self, draft: Draft) {
        if self.steps.len() >= MAX_STEPS {
            return;
        }
        self.steps.push(draft);
    }

    fn display(&self) -> Vec<PlanStep> {
        self.steps
            .iter()
            .map(|step| {
                let text = if step.state == PlanStepState::Doing && !step.doing.is_empty() {
                    step.doing.clone()
                } else {
                    step.text.clone()
                };
                PlanStep {
                    text,
                    state: step.state,
                }
            })
            .collect()
    }
}

pub(crate) fn tool_carries_plan(name: &str) -> bool {
    matches!(
        normalize_tool_name(name).as_str(),
        "todowrite"
            | "todoread"
            | "updateplan"
            | "writeplan"
            | "createplan"
            | "plan"
            | "todolist"
            | "taskcreate"
            | "taskupdate"
            | "tasklist"
    )
}

/// Checklist from a tool name plus its input. Ordinary tools return `None`.
pub(crate) fn plan_from_tool(name: &str, input: &Value) -> Option<Vec<PlanStep>> {
    let mut board = PlanBoard::default();
    let drafts = drafts_from_tool(name, input)?;
    board.steps = drafts;
    let shown = board.display();
    if shown.is_empty() {
        None
    } else {
        Some(shown)
    }
}

/// Checklist hiding inside a Codex item, including nested tool arguments.
pub(crate) fn plan_from_item(item: &Value) -> Option<Vec<PlanStep>> {
    let type_name = item.get("type").and_then(Value::as_str).unwrap_or("");
    if let Some(steps) = plan_from_tool(type_name, item) {
        return Some(steps);
    }
    let tool_name = ["tool", "name", "toolName", "tool_name"]
        .iter()
        .find_map(|key| item.get(*key).and_then(Value::as_str))
        .unwrap_or("");
    for key in ["arguments", "input", "params", "rawInput", "raw_input"] {
        let Some(nested) = item.get(key) else {
            continue;
        };
        if let Some(steps) = plan_from_tool(tool_name, nested) {
            return Some(steps);
        }
        let Some(text) = nested.as_str() else {
            continue;
        };
        let Ok(parsed) = serde_json::from_str::<Value>(text) else {
            continue;
        };
        if let Some(steps) = plan_from_tool(tool_name, &parsed) {
            return Some(steps);
        }
    }
    None
}

fn drafts_from_tool(name: &str, input: &Value) -> Option<Vec<Draft>> {
    if !tool_carries_plan(name) && !input_has_plan_list(input) {
        return None;
    }
    drafts_from_value(input)
}

fn input_has_plan_list(input: &Value) -> bool {
    let Some(obj) = input.as_object() else {
        return false;
    };
    ["todo", "todos", "plan"].iter().any(|key| {
        obj.get(*key)
            .and_then(Value::as_array)
            .is_some_and(|list| !list.is_empty())
    })
}

fn drafts_from_value(input: &Value) -> Option<Vec<Draft>> {
    let obj = input.as_object()?;
    for key in ["todo", "todos", "plan", "steps", "items", "tasks"] {
        let Some(list) = obj.get(key).and_then(Value::as_array) else {
            continue;
        };
        let drafts: Vec<Draft> = list
            .iter()
            .filter_map(draft_from_value)
            .take(MAX_STEPS)
            .collect();
        if !drafts.is_empty() {
            return Some(drafts);
        }
    }
    None
}

fn draft_from_value(value: &Value) -> Option<Draft> {
    let obj = value.as_object()?;
    let marked =
        obj.contains_key("status") || obj.contains_key("state") || obj.contains_key("completed");
    if !marked {
        return None;
    }
    let text = first_string(
        value,
        &[
            "content",
            "text",
            "step",
            "subject",
            "title",
            "task",
            "description",
        ],
    );
    let doing = first_string(value, &["activeForm", "active_form"]);
    let text = if text.is_empty() { doing.clone() } else { text };
    if text.is_empty() {
        return None;
    }
    let state = if obj.get("completed").and_then(Value::as_bool) == Some(true) {
        PlanStepState::Done
    } else {
        map_state(&first_string(value, &["status", "state"]))
    };
    Some(Draft {
        id: first_string(value, &["id", "taskId", "task_id"]),
        text: clip_chars(&text, MAX_TEXT),
        doing: clip_chars(&doing, MAX_TEXT),
        state,
    })
}

fn task_id_from_result(result: Option<&Value>) -> Option<String> {
    let result = result?;
    ["task/id", "task/taskId", "id", "taskId"]
        .iter()
        .find_map(|pointer| result.pointer(&format!("/{pointer}")).and_then(json_id))
}

fn json_id(value: &Value) -> Option<String> {
    match value {
        Value::String(text) if !text.is_empty() => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn first_string(value: &Value, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .unwrap_or("")
        .to_string()
}

fn map_state(raw: &str) -> PlanStepState {
    let normalized = normalize_tool_name(raw);
    match normalized.as_str() {
        "completed" | "complete" | "done" | "finished" | "success" => PlanStepState::Done,
        "inprogress" | "doing" | "active" | "current" | "running" | "started" => {
            PlanStepState::Doing
        }
        _ => PlanStepState::Pending,
    }
}

fn normalize_tool_name(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

fn clip_chars(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn claude_todo_write_becomes_a_checklist() {
        let input = json!({
            "todos": [
                {"content": "看登录", "status": "completed", "activeForm": "看着登录"},
                {"content": "改按钮", "status": "in_progress", "activeForm": "改着按钮"},
                {"content": "再试一次", "status": "pending", "activeForm": "再试"}
            ]
        });
        let steps = plan_from_tool("TodoWrite", &input).expect("plan");
        assert_eq!(
            steps,
            vec![
                PlanStep {
                    text: "看登录".into(),
                    state: PlanStepState::Done
                },
                PlanStep {
                    text: "改着按钮".into(),
                    state: PlanStepState::Doing
                },
                PlanStep {
                    text: "再试一次".into(),
                    state: PlanStepState::Pending
                },
            ]
        );
    }

    #[test]
    fn newer_claude_tasks_patch_one_step() {
        let mut board = PlanBoard::default();
        let created = board
            .observe_tool(
                "TaskCreate",
                "toolu_1",
                &json!({"subject": "改按钮", "activeForm": "正在改按钮"}),
            )
            .expect("create");
        assert_eq!(created[0].text, "改按钮");
        assert_eq!(created[0].state, PlanStepState::Pending);

        board.observe_user(&json!({
            "type": "user",
            "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "toolu_1"}]},
            "tool_use_result": {"task": {"id": "3", "subject": "改按钮"}}
        }));
        let updated = board
            .observe_tool(
                "TaskUpdate",
                "toolu_2",
                &json!({"taskId": "3", "status": "in_progress", "activeForm": "正在改按钮"}),
            )
            .expect("update");
        assert_eq!(updated[0].text, "正在改按钮");
        assert_eq!(updated[0].state, PlanStepState::Doing);

        let cleared = board
            .observe_tool(
                "TaskUpdate",
                "toolu_3",
                &json!({"taskId": "3", "status": "deleted"}),
            )
            .expect("delete");
        assert!(cleared.is_empty());
    }

    #[test]
    fn codex_update_plan_and_turn_notification() {
        let input = json!({
            "explanation": "先列出来",
            "plan": [
                {"step": "读文件", "status": "completed"},
                {"step": "改文件", "status": "in_progress"}
            ]
        });
        let steps = plan_from_tool("update_plan", &input).expect("plan");
        assert_eq!(steps[1].text, "改文件");
        assert_eq!(steps[1].state, PlanStepState::Doing);

        let item = json!({
            "type": "mcpToolCall",
            "tool": "todo_write",
            "arguments": input.to_string()
        });
        assert!(plan_from_item(&item).is_some());

        let notice = json!({
            "turnId": "t1",
            "todo": [
                {"step": "读文件", "status": "completed"},
                {"step": "改文件", "status": "inProgress"}
            ],
            "plan": [
                {"step": "读文件", "status": "completed"},
                {"step": "改文件", "status": "inProgress"}
            ]
        });
        let from_turn = plan_from_tool("update_plan", &notice).expect("turn");
        assert_eq!(from_turn[1].state, PlanStepState::Doing);
    }

    #[test]
    fn plan_item_and_todo_list() {
        let plan = json!({
            "type": "plan",
            "items": [
                {"text": "第一步", "status": "done"},
                {"text": "第二步", "status": "pending"}
            ]
        });
        assert_eq!(plan_from_item(&plan).expect("plan").len(), 2);

        let todos = json!({
            "type": "todo_list",
            "items": [
                {"text": "已完成", "completed": true},
                {"text": "还没做", "completed": false}
            ]
        });
        let steps = plan_from_item(&todos).expect("todos");
        assert_eq!(steps[0].state, PlanStepState::Done);
        assert_eq!(steps[1].state, PlanStepState::Pending);
    }

    #[test]
    fn ordinary_tools_and_plan_prose_are_not_checklists() {
        assert!(plan_from_tool("Bash", &json!({"command": "ls"})).is_none());
        assert!(plan_from_tool("Read", &json!({"file_path": "a.ts"})).is_none());
        assert!(plan_from_item(&json!({"type": "commandExecution", "command": "ls"})).is_none());
        assert!(plan_from_tool("TodoWrite", &json!({})).is_none());
        assert!(
            plan_from_item(&json!({"type": "plan", "id": "p1", "text": "先改登录，再改按钮"}))
                .is_none()
        );
    }

    #[test]
    fn clips_long_lists_and_text() {
        let todos: Vec<_> = (0..40)
            .map(
                |i| json!({"content": format!("步骤{i}{}", "很".repeat(200)), "status": "pending"}),
            )
            .collect();
        let steps = plan_from_tool("TodoWrite", &json!({"todos": todos})).expect("plan");
        assert_eq!(steps.len(), MAX_STEPS);
        assert!(steps[0].text.chars().count() <= MAX_TEXT);
        assert!(steps[0].text.ends_with('…'));
    }
}
