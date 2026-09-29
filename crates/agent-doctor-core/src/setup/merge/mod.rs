mod ask_tools;
mod claude;
mod codex;
mod hermes;
mod openclaw;

pub use ask_tools::*;
pub use claude::*;
pub use codex::*;
pub use hermes::*;
pub use openclaw::*;

/// Legacy single-slot id (migrated away; removed when writing additive slots).
pub const OPENCLAW_PROVIDER_ID: &str = "agent-doctor";
/// Team / Evotown slot in OpenClaw `models.providers` (additive mode).
pub const OPENCLAW_TEAM_SLOT: &str = "evotown";
/// Personal slot in OpenClaw `models.providers` (additive mode).
pub const OPENCLAW_PERSONAL_SLOT: &str = "personal";

/// Codex `model_providers` team slot (pointer via `model_provider`).
pub const CODEX_TEAM_SLOT: &str = "company";
/// Codex `model_providers` personal slot.
pub const CODEX_PERSONAL_SLOT: &str = "personal";

/// Hermes additive slot ids (sidecar + active `model.*` pointer).
pub const HERMES_TEAM_SLOT: &str = "company";
pub const HERMES_PERSONAL_SLOT: &str = "personal";

/// Default model id for company/Evotown wiring when none is specified.
/// Prefer a gateway-routable id: bare `default` / `gpt-4o-*` often 502/503 upstream.
pub const COMPANY_DEFAULT_MODEL: &str = "deepseek-v4-flash";
