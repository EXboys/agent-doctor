//! Node.js engine compatibility, shared by every npm-installed runtime.
//!
//! An npm package that needs a newer Node than the one on PATH does not always
//! fail loudly. `@deepseek-ai/dsh` on Node 22.12 exits 0 with no output at all,
//! because `import.meta.main` is `undefined` below 22.18, so `bin.js` skips its
//! own `runCli()` call. Nothing on the outside looks broken: the binary exists,
//! `--version` "succeeds", and chat just never answers.
//!
//! The package itself knows the answer — `engines.node` in its `package.json` —
//! so the check reads that instead of hard-coding a floor per runtime.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::exec::run_output;
use crate::repair_types::{DiagnosticFact, SensitivityLevel};
use crate::version_check::{extract_version, version_is_older};

use super::{ProbeCheck, ProbeSeverity, ProbeStatus};

const NODE_VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// The `engines.node` range an installed package declares, if it declares one.
pub fn read_engine_range(package_json: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(package_json).ok()?;
    let manifest = serde_json::from_str::<serde_json::Value>(&raw).ok()?;
    let range = manifest.get("engines")?.get("node")?.as_str()?.trim();
    if range.is_empty() || range == "*" {
        None
    } else {
        Some(range.to_string())
    }
}

/// Lowest version a range accepts, as `major.minor.patch`.
///
/// Handles the spellings that actually ship in `engines.node`: `>=24.9.0`,
/// `^22.19.0`, `~20.11`, bare `18`, and `||` alternates. Upper bounds (`<`, `<=`)
/// are skipped — only the floor can make an install unusable.
pub fn minimum_from_range(range: &str) -> Option<String> {
    let mut lowest: Option<Vec<u64>> = None;
    for alternative in range.split("||") {
        let mut floor_in_alternative: Option<Vec<u64>> = None;
        for token in alternative.split_whitespace() {
            let token = token.trim();
            if token.is_empty() || token.starts_with('<') {
                continue;
            }
            let bare = token
                .trim_start_matches(">=")
                .trim_start_matches('>')
                .trim_start_matches('^')
                .trim_start_matches('~')
                .trim_start_matches('=')
                .trim();
            let Some(version) = parse_semver_floor(bare) else {
                continue;
            };
            if floor_in_alternative
                .as_ref()
                .map_or(true, |current| version < *current)
            {
                floor_in_alternative = Some(version);
            }
        }
        if let Some(version) = floor_in_alternative {
            if lowest.as_ref().map_or(true, |current| version < *current) {
                lowest = Some(version);
            }
        }
    }
    lowest.map(|parts| {
        parts
            .iter()
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join(".")
    })
}

fn parse_semver_floor(raw: &str) -> Option<Vec<u64>> {
    let core = raw.split(['-', '+']).next()?.trim();
    if core.is_empty() || core.starts_with('*') || core.contains('x') || core.contains('X') {
        return None;
    }
    let mut parts = Vec::with_capacity(3);
    for segment in core.split('.') {
        let digits: String = segment.chars().take_while(char::is_ascii_digit).collect();
        if digits.is_empty() {
            break;
        }
        parts.push(digits.parse().ok()?);
        if parts.len() == 3 {
            break;
        }
    }
    if parts.is_empty() {
        return None;
    }
    while parts.len() < 3 {
        parts.push(0);
    }
    Some(parts)
}

/// Version of the `node` this process would launch — the same one a runtime's
/// CLI gets, which is the whole point of the check.
pub fn node_version_on_path() -> Option<String> {
    let binary = crate::adapters::util::find_binary("node")?;
    let output = run_output(&binary, &["--version"], NODE_VERSION_PROBE_TIMEOUT).ok()?;
    if !output.status.success() {
        return None;
    }
    let raw = String::from_utf8_lossy(&output.stdout);
    extract_version(raw.trim())
}

/// Path to an installed npm package's `package.json`, by walking up from the
/// binary's directory until `node_modules/<package>` shows up.
pub fn package_json_for_binary(binary_name: &str, package_name: &str) -> Option<PathBuf> {
    let binary = crate::adapters::util::find_binary(binary_name)?;
    let mut dir = binary.parent()?;
    for _ in 0..4 {
        for candidate in [
            dir.join("node_modules")
                .join(package_name)
                .join("package.json"),
            dir.join("lib")
                .join("node_modules")
                .join(package_name)
                .join("package.json"),
        ] {
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        dir = dir.parent()?;
    }
    None
}

/// Emit `node.engine` checks/facts for one runtime.
///
/// The floor comes from the installed package's `engines.node` when it declares
/// one; `fallback_minimum` covers packages that ship without an `engines` field
/// (`@deepseek-ai/dsh@0.1.5` is one) but still use APIs with a known Node floor.
/// Pass `None` for runtimes that also ship a native binary — a missing manifest
/// there means no Node is involved, so there is nothing to check.
///
/// Silent when no Node can be found at all: the runtime's own `binary.exists`
/// check already reports that.
pub fn probe_node_engine(
    runtime_id: &str,
    manifest: Option<&Path>,
    fallback_minimum: Option<&str>,
    checks: &mut Vec<ProbeCheck>,
    facts: &mut Vec<DiagnosticFact>,
) {
    let declared = manifest.and_then(read_engine_range);
    let floor = declared
        .as_deref()
        .and_then(minimum_from_range)
        .or_else(|| fallback_minimum.map(str::to_string));
    let Some(minimum) = floor else {
        return;
    };
    let Some(actual) = node_version_on_path() else {
        return;
    };
    let range = declared.unwrap_or_else(|| format!(">={minimum}"));

    facts.push(DiagnosticFact::new(
        format!("{runtime_id}.node.required"),
        minimum.clone(),
        SensitivityLevel::Public,
    ));
    facts.push(DiagnosticFact::new(
        format!("{runtime_id}.node.actual"),
        actual.clone(),
        SensitivityLevel::Public,
    ));

    let id = format!("{runtime_id}.node.engine");
    if version_is_older(&actual, &minimum) {
        checks.push(ProbeCheck::new(
            id,
            "Node.js version",
            ProbeStatus::Fail,
            ProbeSeverity::Error,
            format!(
                "Node.js {actual} is older than {minimum}, which this package requires \
                 ({range}); its CLI can exit without starting"
            ),
            SensitivityLevel::Public,
        ));
    } else {
        checks.push(ProbeCheck::new(
            id,
            "Node.js version",
            ProbeStatus::Pass,
            ProbeSeverity::Info,
            format!("Node.js {actual} satisfies {range}"),
            SensitivityLevel::Public,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_greater_or_equal_floor() {
        assert_eq!(minimum_from_range(">=24.9.0").as_deref(), Some("24.9.0"));
    }

    #[test]
    fn reads_caret_and_tilde_floors() {
        assert_eq!(minimum_from_range("^22.19.0").as_deref(), Some("22.19.0"));
        assert_eq!(minimum_from_range("~20.11").as_deref(), Some("20.11.0"));
    }

    #[test]
    fn bare_major_is_a_floor() {
        assert_eq!(minimum_from_range("18").as_deref(), Some("18.0.0"));
    }

    #[test]
    fn upper_bounds_do_not_set_the_floor() {
        assert_eq!(
            minimum_from_range(">=22.19.0 <25").as_deref(),
            Some("22.19.0")
        );
    }

    #[test]
    fn alternates_take_the_lowest_branch() {
        assert_eq!(
            minimum_from_range("^20.11.0 || >=22.19.0").as_deref(),
            Some("20.11.0")
        );
    }

    #[test]
    fn wildcards_declare_no_floor() {
        assert!(minimum_from_range("*").is_none());
        assert!(minimum_from_range("latest").is_none());
        assert!(minimum_from_range(">=x").is_none());
    }

    #[test]
    fn prerelease_is_compared_by_its_release_core() {
        assert_eq!(
            minimum_from_range(">=24.9.0-rc.1").as_deref(),
            Some("24.9.0")
        );
    }
}
