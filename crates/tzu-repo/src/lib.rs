use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::process::Command;

#[derive(Debug, Error)]
pub enum RepoError {
    #[error("git command failed: {0}")]
    Git(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoState {
    pub root: PathBuf,
    pub head: Option<String>,
    pub dirty: bool,
    pub files: Vec<FileEntry>,
    pub languages: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: PathBuf,
    pub language: Option<String>,
}

pub async fn inspect_repo(root: &Path) -> Result<RepoState, RepoError> {
    let head = git_output(root, ["rev-parse", "--verify", "HEAD"])
        .await
        .ok();
    let dirty = git_output(root, ["status", "--porcelain"])
        .await
        .map(|out| !out.trim().is_empty())
        .unwrap_or(false);
    let mut files = Vec::new();
    let mut languages = BTreeMap::new();
    collect_files(root, root, &mut files, &mut languages)?;
    Ok(RepoState {
        root: root.to_path_buf(),
        head: head.map(|value| value.trim().to_string()),
        dirty,
        files,
        languages,
    })
}

async fn git_output<const N: usize>(root: &Path, args: [&str; N]) -> Result<String, RepoError> {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .await?;
    if !output.status.success() {
        return Err(RepoError::Git(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn collect_files(
    root: &Path,
    dir: &Path,
    files: &mut Vec<FileEntry>,
    languages: &mut BTreeMap<String, usize>,
) -> Result<(), RepoError> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == ".git" || name == "target" || name == ".tzu" {
            continue;
        }
        if path.is_dir() {
            collect_files(root, &path, files, languages)?;
        } else {
            let rel = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
            let language = detect_language(&rel);
            if let Some(language) = language.as_ref() {
                *languages.entry(language.clone()).or_insert(0) += 1;
            }
            files.push(FileEntry {
                path: rel,
                language,
            });
        }
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(())
}

#[must_use]
pub fn detect_language(path: &Path) -> Option<String> {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("rs") => Some("Rust".to_string()),
        Some("nix") => Some("Nix".to_string()),
        Some("toml") => Some("TOML".to_string()),
        Some("md") => Some("Markdown".to_string()),
        Some("json") => Some("JSON".to_string()),
        Some("yaml" | "yml") => Some("YAML".to_string()),
        Some("sql") => Some("SQL".to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_detection_is_extension_based() {
        assert_eq!(
            detect_language(Path::new("src/main.rs")).as_deref(),
            Some("Rust")
        );
        assert_eq!(
            detect_language(Path::new("README.md")).as_deref(),
            Some("Markdown")
        );
        assert_eq!(detect_language(Path::new("LICENSE")), None);
    }
}
