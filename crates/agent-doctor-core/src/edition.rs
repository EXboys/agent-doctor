//! Build-time product edition: personal (TeamUps) vs team (enterprise).
//!
//! Editions are locked at compile/package time. Unlike runtime LLM *mode*,
//! an edition does not switch: personal builds only wire personal providers;
//! team builds only wire Evotown.

use anyhow::{bail, Result};

use crate::setup::{MODE_PERSONAL, MODE_TEAM};

/// Product edition baked into this binary / package.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProductEdition {
    /// TeamUps / consumer: personal provider wiring only.
    Personal,
    /// Enterprise: Evotown / team gateway wiring only.
    Team,
}

impl ProductEdition {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Personal => "personal",
            Self::Team => "team",
        }
    }

    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "team" | "enterprise" | "evotown" => Self::Team,
            _ => Self::Personal,
        }
    }

    pub fn allows_mode(self, mode: &str) -> bool {
        matches!(
            (self, mode),
            (Self::Personal, MODE_PERSONAL) | (Self::Team, MODE_TEAM)
        )
    }
}

/// Resolve the product edition.
///
/// Preference: compile-time `AGENT_DOCTOR_EDITION` (via `option_env!`) so
/// packaged builds cannot be flipped by the user; otherwise runtime env
/// (dev/testing); default **personal**.
pub fn product_edition() -> ProductEdition {
    if let Some(compiled) = option_env!("AGENT_DOCTOR_EDITION") {
        let trimmed = compiled.trim();
        if !trimmed.is_empty() {
            return ProductEdition::parse(trimmed);
        }
    }
    std::env::var("AGENT_DOCTOR_EDITION")
        .ok()
        .map(|v| ProductEdition::parse(&v))
        .unwrap_or(ProductEdition::Personal)
}

/// Refuse LLM wiring that does not belong to this edition.
pub fn ensure_edition_allows_mode(mode: &str) -> Result<()> {
    let edition = product_edition();
    if edition.allows_mode(mode) {
        return Ok(());
    }
    match edition {
        ProductEdition::Personal => bail!(
            "this Agent Doctor build is the personal edition (TeamUps); \
             team / Evotown wiring is not available — use the team/enterprise package"
        ),
        ProductEdition::Team => bail!(
            "this Agent Doctor build is the team edition; \
             personal provider wiring is not available — use the personal/TeamUps package"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_aliases() {
        assert_eq!(ProductEdition::parse("personal"), ProductEdition::Personal);
        assert_eq!(ProductEdition::parse("TEAM"), ProductEdition::Team);
        assert_eq!(ProductEdition::parse("enterprise"), ProductEdition::Team);
        assert_eq!(ProductEdition::parse(""), ProductEdition::Personal);
    }

    #[test]
    fn allows_matching_mode_only() {
        assert!(ProductEdition::Personal.allows_mode(MODE_PERSONAL));
        assert!(!ProductEdition::Personal.allows_mode(MODE_TEAM));
        assert!(ProductEdition::Team.allows_mode(MODE_TEAM));
        assert!(!ProductEdition::Team.allows_mode(MODE_PERSONAL));
    }
}
