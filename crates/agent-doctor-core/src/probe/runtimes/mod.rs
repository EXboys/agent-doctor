pub mod claude_code;
pub mod codex;
pub mod deepseek_harness;
pub mod hermes;
pub mod openclaw;

pub(crate) use claude_code::probe_schema as schema_claude_code;
pub(crate) use codex::probe_schema as schema_codex;
pub(crate) use deepseek_harness::{
    probe_deep as deepseek_harness_probe_deep, probe_schema as schema_deepseek_harness,
};
pub(crate) use hermes::{probe_deep, probe_schema as schema_hermes};
pub(crate) use openclaw::{probe_deep as openclaw_probe_deep, probe_schema as schema_openclaw};
