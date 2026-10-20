use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Result};

use super::policy::path_is_allowed;
use crate::mask::{mask_config_file, SecretVault};

const MAX_LIST_ENTRIES: usize = 200;
const MAX_GREP_MATCHES: usize = 50;
const MAX_GREP_FILE_BYTES: u64 = 256 * 1024;
const MAX_WALK_DEPTH: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryKind {
    File,
    Dir,
}

pub fn list_allowed_directory(path: Option<&Path>, allowed_roots: &[PathBuf]) -> Result<String> {
    let Some(path) = path else {
        return Ok(format_allowed_roots(allowed_roots));
    };

    if !path_is_allowed(path, allowed_roots) {
        bail!("path not allowed: {}", path.display());
    }

    if path.is_file() {
        return Ok(format_file_entry(path, EntryKind::File));
    }

    if !path.is_dir() {
        return Ok(format!("(not found: {})", path.display()));
    }

    list_directory_children(path, allowed_roots)
}

pub fn grep_allowed_files(
    pattern: &str,
    path: Option<&Path>,
    allowed_roots: &[PathBuf],
    vault: &mut SecretVault,
) -> Result<String> {
    if pattern.is_empty() {
        bail!("pattern must not be empty");
    }

    let files = if let Some(path) = path {
        if !path_is_allowed(path, allowed_roots) {
            bail!("path not allowed: {}", path.display());
        }
        collect_files_under(path, allowed_roots, 0)?
    } else {
        let mut files = Vec::new();
        for root in allowed_roots {
            if root.is_file() {
                files.push(root.clone());
            } else if root.is_dir() {
                files.extend(collect_files_under(root, allowed_roots, 0)?);
            }
        }
        files.sort();
        files.dedup();
        files
    };

    let mut matches = Vec::new();
    for file in files {
        if matches.len() >= MAX_GREP_MATCHES {
            break;
        }
        let Ok(meta) = fs::metadata(&file) else {
            continue;
        };
        if meta.len() > MAX_GREP_FILE_BYTES {
            continue;
        }
        let Ok(raw) = fs::read_to_string(&file) else {
            continue;
        };

        for (line_no, line) in raw.lines().enumerate() {
            if matches.len() >= MAX_GREP_MATCHES {
                break;
            }
            if !line.contains(pattern) {
                continue;
            }
            let mut snippet_vault = SecretVault::default();
            let masked_line = mask_config_file(&file, line, &mut snippet_vault);
            vault.absorb(snippet_vault);
            matches.push(format!(
                "{}:{}:{}",
                file.display(),
                line_no + 1,
                masked_line
            ));
        }
    }

    if matches.is_empty() {
        return Ok(format!("(no matches for `{pattern}`)"));
    }

    let mut out = matches.join("\n");
    if matches.len() >= MAX_GREP_MATCHES {
        out.push_str(&format!(
            "\n(truncated at {MAX_GREP_MATCHES} matches — narrow pattern or path)"
        ));
    }
    Ok(out)
}

fn format_allowed_roots(allowed_roots: &[PathBuf]) -> String {
    if allowed_roots.is_empty() {
        return "(no allowed paths registered for this runtime)".to_string();
    }

    let mut lines = vec!["Allowed repair paths:".to_string()];
    for root in allowed_roots {
        if root.is_file() {
            lines.push(format!(
                "  [file] {}",
                format_file_entry(root, EntryKind::File)
            ));
        } else if root.is_dir() {
            lines.push(format!("  [dir]  {}", root.display()));
            if let Ok(children) = list_directory_children(root, allowed_roots) {
                for child in children.lines() {
                    lines.push(format!("         {child}"));
                }
            }
        } else if root.exists() {
            lines.push(format!("  [?]    {}", root.display()));
        } else {
            lines.push(format!("  [missing] {}", root.display()));
        }
    }
    lines.join("\n")
}

fn list_directory_children(path: &Path, allowed_roots: &[PathBuf]) -> Result<String> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(path)? {
        let Ok(entry) = entry else {
            continue;
        };
        let entry_path = entry.path();
        if !path_is_allowed(&entry_path, allowed_roots) {
            continue;
        }
        entries.push(entry_path);
        if entries.len() >= MAX_LIST_ENTRIES {
            break;
        }
    }
    entries.sort();

    if entries.is_empty() {
        return Ok(format!("(empty directory: {})", path.display()));
    }

    let mut lines = Vec::new();
    for entry_path in entries {
        let kind = if entry_path.is_dir() {
            EntryKind::Dir
        } else {
            EntryKind::File
        };
        lines.push(format_entry_line(&entry_path, kind));
        if lines.len() >= MAX_LIST_ENTRIES {
            lines.push(format!(
                "(truncated at {MAX_LIST_ENTRIES} entries — use a narrower path)"
            ));
            break;
        }
    }
    Ok(lines.join("\n"))
}

fn collect_files_under(
    path: &Path,
    allowed_roots: &[PathBuf],
    depth: usize,
) -> Result<Vec<PathBuf>> {
    if depth > MAX_WALK_DEPTH {
        return Ok(Vec::new());
    }

    if path.is_file() {
        return Ok(vec![path.to_path_buf()]);
    }

    if !path.is_dir() {
        return Ok(Vec::new());
    }

    let mut files = Vec::new();
    for entry in fs::read_dir(path)? {
        let Ok(entry) = entry else {
            continue;
        };
        let entry_path = entry.path();
        if !path_is_allowed(&entry_path, allowed_roots) {
            continue;
        }
        if entry_path.is_dir() {
            files.extend(collect_files_under(&entry_path, allowed_roots, depth + 1)?);
        } else if entry_path.is_file() {
            files.push(entry_path);
        }
    }
    Ok(files)
}

fn format_file_entry(path: &Path, kind: EntryKind) -> String {
    format_entry_line(path, kind)
}

fn format_entry_line(path: &Path, kind: EntryKind) -> String {
    let label = match kind {
        EntryKind::File => "file",
        EntryKind::Dir => "dir",
    };
    let size = fs::metadata(path).map(|meta| meta.len()).unwrap_or(0);
    format!("{label} {} ({size} bytes)", path.display())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn lists_allowed_directory_children() {
        let dir = TempDir::new().expect("tempdir");
        let allowed = vec![dir.path().to_path_buf()];
        std::fs::write(
            dir.path().join("config.yaml"),
            "model:\n  provider: openai\n",
        )
        .expect("write");

        let listing = list_allowed_directory(Some(dir.path()), &allowed).expect("list");
        assert!(listing.contains("config.yaml"));
    }

    #[test]
    fn grep_finds_masked_matches() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join(".env");
        let mut file = std::fs::File::create(&path).expect("create");
        writeln!(file, "DEEPSEEK_API_KEY=sk-test-secret-key").expect("write");
        let allowed = vec![dir.path().to_path_buf()];
        let mut vault = SecretVault::default();

        let output =
            grep_allowed_files("DEEPSEEK", Some(dir.path()), &allowed, &mut vault).expect("grep");
        assert!(output.contains("DEEPSEEK"));
        assert!(output.contains("{{SECRET:"));
        assert!(!output.contains("sk-test-secret-key"));
    }

    #[test]
    fn rejects_grep_outside_allowed_roots() {
        let dir = TempDir::new().expect("tempdir");
        let outside = std::env::temp_dir().join("agent-doctor-grep-outside");
        let allowed = vec![dir.path().to_path_buf()];
        let mut vault = SecretVault::default();
        let err = grep_allowed_files("foo", Some(&outside), &allowed, &mut vault).unwrap_err();
        assert!(err.to_string().contains("not allowed"));
    }
}
