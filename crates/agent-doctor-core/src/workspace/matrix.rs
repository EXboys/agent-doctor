use serde::Serialize;

use crate::runtime::{all_runtime_ids, descriptor_by_id};

#[derive(Debug, Clone, Serialize)]
pub struct CapabilityCell {
    pub runtime: &'static str,
    pub dimension: &'static str,
    pub native: &'static str,
    pub agent_doctor: &'static str,
    pub tier: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct CapabilityMatrix {
    pub version: &'static str,
    pub rows: Vec<CapabilityCell>,
}

pub fn workspace_capability_matrix() -> CapabilityMatrix {
    let mut rows = workspace_capability_rows();
    for id in all_runtime_ids() {
        let Some(descriptor) = descriptor_by_id(id) else {
            continue;
        };
        if descriptor.supports_workspace_bind() {
            continue;
        }
        rows.push(CapabilityCell {
            runtime: id,
            dimension: "Project isolation",
            native: "No per-project bind",
            agent_doctor: "No workspace bind on this runtime",
            tier: "—",
        });
    }
    CapabilityMatrix {
        version: "workspace-v1.6",
        rows,
    }
}

fn workspace_capability_rows() -> Vec<CapabilityCell> {
    vec![
        CapabilityCell {
            runtime: "Hermes",
            dimension: "Memory / sessions",
            native: "Per profile (~/.hermes/profiles/<name>)",
            agent_doctor: "Dedicated profile per workspace + HERMES_HOME",
            tier: "L3",
        },
        CapabilityCell {
            runtime: "Hermes",
            dimension: "Gateway",
            native: "One gateway.lock per running profile",
            agent_doctor: "Doctor detects mismatch; --restart-gateways",
            tier: "L3",
        },
        CapabilityCell {
            runtime: "Claude Code",
            dimension: "Memory",
            native: "Per project hash (~/.claude/projects/)",
            agent_doctor: "Project .claude/ scaffold + cwd alignment",
            tier: "L3",
        },
        CapabilityCell {
            runtime: "Claude Code",
            dimension: "MCP",
            native: "User, project .mcp.json, ~/.claude.json projects[]",
            agent_doctor: "Snapshot/restore .mcp.json; doctor detects global bleed",
            tier: "L3",
        },
        CapabilityCell {
            runtime: "Claude Code",
            dimension: "Skills",
            native: "Project .claude/skills/",
            agent_doctor: "Snapshot project + Hermes profile skills",
            tier: "L3",
        },
        CapabilityCell {
            runtime: "Codex",
            dimension: "Memory",
            native: "Single ~/.codex (no native per-repo)",
            agent_doctor: "Per-workspace CODEX_HOME overlay",
            tier: "L2",
        },
        CapabilityCell {
            runtime: "OpenClaw",
            dimension: "Agent workspace",
            native: "agents.list[].workspace in openclaw.json",
            agent_doctor: "Bind agent workspace dir per project",
            tier: "L2",
        },
        CapabilityCell {
            runtime: "OpenClaw",
            dimension: "Default routing",
            native: "agents.list default + bindings",
            agent_doctor: "Sets default=true + agents.defaults.workspace on use/fix",
            tier: "L2",
        },
        CapabilityCell {
            runtime: "OpenClaw",
            dimension: "MCP / skills paths",
            native: "Config path references",
            agent_doctor: "Doctor scans missing refs; fix re-binds routing",
            tier: "L2",
        },
        CapabilityCell {
            runtime: "Claude Code",
            dimension: "Global MCP migration",
            native: "Manual only",
            agent_doctor: "fix scaffolds .mcp.json + writes migration hint",
            tier: "L3",
        },
        CapabilityCell {
            runtime: "Codex",
            dimension: "Isolation guard",
            native: "None",
            agent_doctor: "CODEX_HOME marker + doctor fails on ~/.codex alias",
            tier: "L2",
        },
        CapabilityCell {
            runtime: "Cross-runtime",
            dimension: "Company baseline drift",
            native: "None",
            agent_doctor: "Doctor compares profile.env gateway vs Hermes/OpenClaw",
            tier: "—",
        },
        CapabilityCell {
            runtime: "Cross-runtime",
            dimension: "Shell env",
            native: "Manual export",
            agent_doctor: "active-workspace.env + zsh/bash/fish hooks + direnv",
            tier: "—",
        },
        CapabilityCell {
            runtime: "Cross-runtime",
            dimension: "Switch safety",
            native: "None",
            agent_doctor: "Backup before use; workspace fix playbook",
            tier: "—",
        },
        CapabilityCell {
            runtime: "Cross-runtime",
            dimension: "Desktop",
            native: "N/A",
            agent_doctor: "Tray tooltip + workspace picker",
            tier: "—",
        },
    ]
}
