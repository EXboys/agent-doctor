//! Lightweight plan tool for the official agent loop.
//!
//! Ported from SkillLite's planning-control design: a typed control tool the
//! loop dispatches by enum (never by string compare), with completed steps
//! preserved across updates and a bounded number of replans.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::text;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlanStep {
    pub title: String,
    #[serde(default)]
    pub completed: bool,
}

#[derive(Debug, Default)]
pub struct PlanState {
    pub steps: Vec<PlanStep>,
    replans: usize,
}

/// SkillLite caps replans per session; past the cap the plan still updates but
/// the result warns the model to stop replanning.
const MAX_REPLANS: usize = 3;

pub const UPDATE_PLAN_TOOL_NAME: &str = "update_plan";

/// Typed planning-control dispatch: the loop matches on this enum once.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlanningControl {
    UpdatePlan,
}

pub fn planning_control_for(tool_name: &str) -> Option<PlanningControl> {
    match tool_name {
        UPDATE_PLAN_TOOL_NAME => Some(PlanningControl::UpdatePlan),
        _ => None,
    }
}

pub fn update_plan_tool_definition() -> Value {
    json!({
        "type": "function",
        "function": {
            "name": UPDATE_PLAN_TOOL_NAME,
            "description": "Publish or update the plan steps shown to the user. Use it when the request needs several steps, and whenever the plan changes. Completed steps stay done; do not repeat them as new work.",
            "parameters": {
                "type": "object",
                "properties": {
                    "steps": {
                        "type": "array",
                        "description": "Full step list in execution order",
                        "items": {
                            "type": "object",
                            "properties": {
                                "title": { "type": "string" },
                                "status": { "type": "string", "enum": ["pending", "done"] }
                            },
                            "required": ["title"]
                        }
                    },
                    "note": { "type": "string", "description": "One-line reason for the update" }
                },
                "required": ["steps"]
            }
        }
    })
}

impl PlanState {
    /// Merge an incoming step list: the completed prefix survives untouched,
    /// incoming steps replace the pending tail (SkillLite semantics: completed
    /// tasks are preserved, new work is appended after them).
    pub fn apply_update(&mut self, incoming: Vec<PlanStep>) -> String {
        self.replans += 1;
        let mut ack = json!({ "success": true, "steps": self.steps_for_ack(incoming) });
        if self.replans > MAX_REPLANS {
            ack["warning"] = json!(format!(
                "You have updated the plan {} times. Stop replanning; execute or answer.",
                self.replans
            ));
        }
        text::head_tail(&ack.to_string(), text::TOOL_RESULT_MAX_CHARS)
    }

    /// Preserve the completed prefix, then replace the pending tail with the
    /// incoming list — never duplicating or un-completing finished work.
    fn steps_for_ack(&mut self, incoming: Vec<PlanStep>) -> Vec<PlanStep> {
        let mut merged: Vec<PlanStep> = self
            .steps
            .iter()
            .filter(|step| step.completed)
            .cloned()
            .collect();
        for step in incoming {
            let already_done = merged
                .iter()
                .any(|done| done.title == step.title && done.completed);
            if !already_done {
                merged.push(step);
            }
        }
        self.steps = merged;
        self.steps.clone()
    }

    pub fn mark_completed(&mut self, title: &str) {
        if let Some(step) = self.steps.iter_mut().find(|step| step.title == title) {
            step.completed = true;
        }
    }

    pub fn replans(&self) -> usize {
        self.replans
    }
}

/// Parse the `steps` array from raw tool arguments, accepting both real arrays
/// and a stringified JSON array (models do both).
pub fn parse_plan_steps(arguments: &str) -> Result<Vec<PlanStep>, String> {
    let value: Value = serde_json::from_str(arguments)
        .map_err(|error| format!("update_plan arguments must be JSON: {error}"))?;
    let steps_value = match value.get("steps") {
        Some(Value::String(raw)) => serde_json::from_str::<Value>(raw)
            .map_err(|error| format!("steps string is not JSON: {error}"))?,
        Some(parsed) => parsed.clone(),
        None => return Err("update_plan requires steps".to_string()),
    };
    let Value::Array(items) = steps_value else {
        return Err("update_plan steps must be an array".to_string());
    };
    let mut steps = Vec::new();
    for item in items {
        let Some(title) = item.get("title").and_then(Value::as_str) else {
            return Err("each step needs a title".to_string());
        };
        let status = item.get("status").and_then(Value::as_str);
        steps.push(PlanStep {
            title: title.to_string(),
            completed: matches!(status, Some("done" | "completed" | "complete")),
        });
    }
    if steps.is_empty() {
        return Err("update_plan steps must not be empty".to_string());
    }
    Ok(steps)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(title: &str, completed: bool) -> PlanStep {
        PlanStep {
            title: title.to_string(),
            completed,
        }
    }

    #[test]
    fn completed_prefix_survives_and_pending_tail_is_replaced() {
        let mut plan = PlanState::default();
        plan.apply_update(vec![step("backup", false), step("edit config", false)]);
        plan.mark_completed("backup");
        let ack = plan.apply_update(vec![step("rewrite", false)]);
        let parsed: Value = serde_json::from_str(&ack).unwrap();
        let titles: Vec<&str> = parsed["steps"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["title"].as_str().unwrap())
            .collect();
        assert_eq!(titles, vec!["backup", "rewrite"]);
        assert_eq!(parsed["steps"][0]["completed"], true);
    }

    #[test]
    fn replans_are_capped_with_a_warning() {
        let mut plan = PlanState::default();
        for _ in 0..MAX_REPLANS {
            plan.apply_update(vec![step("a", false)]);
        }
        let late = plan.apply_update(vec![step("b", false)]);
        assert!(late.contains("Stop replanning"));
        assert!(plan.replans() > MAX_REPLANS);
    }

    #[test]
    fn model_cannot_uncomplete_steps_or_double_them() {
        let mut plan = PlanState::default();
        plan.apply_update(vec![step("backup", false)]);
        plan.mark_completed("backup");
        plan.apply_update(vec![step("backup", false), step("backup", true)]);
        assert_eq!(plan.steps.len(), 1, "done step is kept exactly once");
        assert!(plan.steps[0].completed);
    }

    #[test]
    fn parses_stringified_steps_and_rejects_garbage() {
        let steps = parse_plan_steps(
            r#"{"steps": "[{\"title\":\"a\",\"status\":\"done\"},{\"title\":\"b\"}]"}"#,
        )
        .unwrap();
        assert_eq!(steps.len(), 2);
        assert!(steps[0].completed);
        assert!(!steps[1].completed);
        assert!(parse_plan_steps("{}").is_err());
        assert!(parse_plan_steps(r#"{"steps": []}"#).is_err());
    }

    #[test]
    fn control_dispatch_is_typed() {
        assert_eq!(
            planning_control_for("update_plan"),
            Some(PlanningControl::UpdatePlan)
        );
        assert_eq!(planning_control_for("read_file"), None);
    }
}
