use std::path::Path;

use crate::repair_types::SensitivityLevel;

use super::{ProbeCheck, ProbeSeverity, ProbeStatus};

pub(crate) fn schema_warn(path: &Path, message: String) -> ProbeCheck {
    ProbeCheck::new(
        format!("config.schema:{}", path.display()),
        "Config schema",
        ProbeStatus::Warn,
        ProbeSeverity::Warning,
        message,
        SensitivityLevel::ConfigShape,
    )
}

pub(crate) fn schema_error(path: &Path, message: impl Into<String>) -> ProbeCheck {
    ProbeCheck::new(
        format!("config.schema:{}", path.display()),
        "Config schema",
        ProbeStatus::Fail,
        ProbeSeverity::Error,
        message,
        SensitivityLevel::ConfigShape,
    )
}
