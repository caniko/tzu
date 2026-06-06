use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

use content_inspector::{ContentType, inspect};
use ignore::{DirEntry, WalkBuilder};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::process::Command;

const DEFAULT_MAX_FILES: usize = 10_000;
const DEFAULT_MAX_BYTES_PER_SNIPPET: usize = 8 * 1024;
const MAX_EXPANSION_BYTES: usize = 64 * 1024;

#[derive(Debug, Error)]
pub enum RepoError {
    #[error("git command failed: {0}")]
    Git(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("walk: {0}")]
    Walk(String),
    #[error("context root `{0}` is unavailable: {1}")]
    ContextRoot(String, String),
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
    #[serde(default)]
    pub size_bytes: u64,
    #[serde(default)]
    pub modified_unix_secs: Option<u64>,
    #[serde(default)]
    pub is_binary: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InspectOptions {
    pub context_roots: Vec<PathBuf>,
    pub include_nested_contexts: bool,
    pub max_files: usize,
    pub max_bytes_per_snippet: usize,
}

impl Default for InspectOptions {
    fn default() -> Self {
        Self {
            context_roots: Vec::new(),
            include_nested_contexts: false,
            max_files: DEFAULT_MAX_FILES,
            max_bytes_per_snippet: DEFAULT_MAX_BYTES_PER_SNIPPET,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectContextSnapshot {
    pub id: String,
    pub state_root: PathBuf,
    pub roots: Vec<ContextRootSnapshot>,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextRootSnapshot {
    pub id: String,
    pub requested_root: PathBuf,
    pub root: PathBuf,
    pub head: Option<String>,
    pub dirty: bool,
    pub files: Vec<FileEntry>,
    pub languages: BTreeMap<String, usize>,
    pub manifests: Vec<ContextDocument>,
    pub docs: Vec<ContextDocument>,
    pub boundaries: Vec<ContextBoundary>,
    pub traversal: TraversalStats,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextDocument {
    pub path: PathBuf,
    pub kind: ContextDocumentKind,
    pub language: Option<String>,
    pub size_bytes: u64,
    pub snippet: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ContextDocumentKind {
    Manifest,
    Documentation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextBoundary {
    pub path: PathBuf,
    pub kind: ContextBoundaryKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ContextBoundaryKind {
    NestedGitRepository,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraversalStats {
    pub traversed_entries: usize,
    pub indexed_files: usize,
    pub skipped_ignored_entries: Option<usize>,
    pub walk_errors: usize,
    pub skipped_nested_contexts: usize,
    pub skipped_after_limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextExpansionRequest {
    pub snapshot_id: String,
    pub root_id: String,
    pub paths: Vec<PathBuf>,
    pub max_bytes_per_file: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextExpansion {
    pub snapshot_id: String,
    pub root_id: String,
    pub files: Vec<ExpandedContextFile>,
    pub blockers: Vec<ContextBlocker>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpandedContextFile {
    pub path: PathBuf,
    pub language: Option<String>,
    pub size_bytes: u64,
    pub truncated: bool,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextBlocker {
    pub path: PathBuf,
    pub reason: ContextBlockerReason,
    pub description: String,
    pub producer: String,
    pub regenerate_command: String,
    pub validation_command: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ContextBlockerReason {
    OutsideRoot,
    NotIndexed,
    Missing,
    Stale,
    Binary,
    Io,
}

pub async fn inspect_repo(root: &Path) -> Result<RepoState, RepoError> {
    let snapshot = inspect_context(root, InspectOptions::default()).await?;
    let Some(primary) = snapshot.roots.into_iter().next() else {
        return Ok(RepoState {
            root: root.to_path_buf(),
            head: None,
            dirty: false,
            files: Vec::new(),
            languages: BTreeMap::new(),
        });
    };
    Ok(RepoState {
        root: primary.root,
        head: primary.head,
        dirty: primary.dirty,
        files: primary.files,
        languages: primary.languages,
    })
}

pub async fn inspect_context(
    state_root: &Path,
    mut options: InspectOptions,
) -> Result<ProjectContextSnapshot, RepoError> {
    if options.context_roots.is_empty() {
        options.context_roots.push(state_root.to_path_buf());
    }
    if options.max_files == 0 {
        options.max_files = DEFAULT_MAX_FILES;
    }
    if options.max_bytes_per_snippet == 0 {
        options.max_bytes_per_snippet = DEFAULT_MAX_BYTES_PER_SNIPPET;
    }

    let state_root = state_root
        .canonicalize()
        .unwrap_or_else(|_| state_root.to_path_buf());
    let mut roots = Vec::new();
    for (idx, requested_root) in options.context_roots.iter().enumerate() {
        roots.push(inspect_context_root(idx + 1, requested_root, &options).await?);
    }
    let summary = summarize_snapshot(&roots);
    Ok(ProjectContextSnapshot {
        id: stable_snapshot_id(&state_root, &roots),
        state_root,
        roots,
        summary,
    })
}

pub fn expand_context_files(
    snapshot: &ProjectContextSnapshot,
    request: ContextExpansionRequest,
) -> ContextExpansion {
    let mut files = Vec::new();
    let mut blockers = Vec::new();
    let max_bytes = request.max_bytes_per_file.clamp(1, MAX_EXPANSION_BYTES);
    let Some(root) = snapshot
        .roots
        .iter()
        .find(|root| root.id == request.root_id)
    else {
        return ContextExpansion {
            snapshot_id: request.snapshot_id,
            root_id: request.root_id,
            files,
            blockers: vec![context_blocker(
                PathBuf::new(),
                ContextBlockerReason::NotIndexed,
                "requested context root id is not present in the snapshot",
                snapshot,
            )],
        };
    };

    let indexed = root
        .files
        .iter()
        .map(|entry| (entry.path.clone(), entry))
        .collect::<BTreeMap<_, _>>();

    for requested_path in request.paths {
        let normalized = normalize_relative_path(&requested_path);
        let Some(rel) = normalized else {
            blockers.push(context_blocker(
                requested_path,
                ContextBlockerReason::OutsideRoot,
                "requested path escapes the context root",
                snapshot,
            ));
            continue;
        };
        let Some(index_entry) = indexed.get(&rel) else {
            blockers.push(context_blocker(
                rel,
                ContextBlockerReason::NotIndexed,
                "requested path was not indexed during the context traversal",
                snapshot,
            ));
            continue;
        };
        let absolute = root.root.join(&rel);
        let metadata = match fs::metadata(&absolute) {
            Ok(metadata) => metadata,
            Err(err) => {
                blockers.push(context_blocker(
                    rel,
                    ContextBlockerReason::Missing,
                    format!("indexed file is no longer readable: {err}"),
                    snapshot,
                ));
                continue;
            }
        };
        let modified = modified_unix_secs(&metadata);
        if metadata.len() != index_entry.size_bytes || modified != index_entry.modified_unix_secs {
            blockers.push(context_blocker(
                rel,
                ContextBlockerReason::Stale,
                "indexed file metadata changed after the context snapshot was created",
                snapshot,
            ));
            continue;
        }
        if index_entry.is_binary {
            blockers.push(context_blocker(
                rel,
                ContextBlockerReason::Binary,
                "indexed file is binary and will not be expanded as text context",
                snapshot,
            ));
            continue;
        }
        match read_text_prefix(&absolute, max_bytes) {
            Ok((content, truncated)) => files.push(ExpandedContextFile {
                path: rel,
                language: index_entry.language.clone(),
                size_bytes: index_entry.size_bytes,
                truncated,
                content,
            }),
            Err(err) => blockers.push(context_blocker(
                rel,
                ContextBlockerReason::Io,
                format!("failed to read indexed file: {err}"),
                snapshot,
            )),
        }
    }

    ContextExpansion {
        snapshot_id: request.snapshot_id,
        root_id: request.root_id,
        files,
        blockers,
    }
}

async fn inspect_context_root(
    index: usize,
    requested_root: &Path,
    options: &InspectOptions,
) -> Result<ContextRootSnapshot, RepoError> {
    let root = requested_root.canonicalize().map_err(|err| {
        RepoError::ContextRoot(requested_root.display().to_string(), err.to_string())
    })?;
    let head = git_output(&root, ["rev-parse", "--verify", "HEAD"])
        .await
        .ok();
    let dirty = git_output(&root, ["status", "--porcelain"])
        .await
        .map(|out| !out.trim().is_empty())
        .unwrap_or(false);

    let mut files = Vec::new();
    let mut languages = BTreeMap::new();
    let mut manifests = Vec::new();
    let mut docs = Vec::new();
    let mut boundaries = Vec::new();
    let mut traversal = TraversalStats::default();
    let mut seen_dirs = BTreeSet::new();
    let skipped_dirs = Arc::new(Mutex::new(BTreeSet::new()));

    let mut builder = WalkBuilder::new(&root);
    let filter_root = root.clone();
    let include_nested_contexts = options.include_nested_contexts;
    let filter_skipped_dirs = Arc::clone(&skipped_dirs);
    builder
        .hidden(false)
        .parents(true)
        .git_ignore(true)
        .git_exclude(true)
        .ignore(true)
        .filter_entry(move |entry| {
            should_descend_entry(
                &filter_root,
                entry,
                include_nested_contexts,
                &filter_skipped_dirs,
            )
        });

    for result in builder.build() {
        let entry = match result {
            Ok(entry) => entry,
            Err(err) => {
                traversal.walk_errors += 1;
                if err.io_error().is_some() {
                    return Err(RepoError::Walk(err.to_string()));
                }
                continue;
            }
        };
        traversal.traversed_entries += 1;
        let path = entry.path();
        if path == root {
            continue;
        }
        let rel = path.strip_prefix(&root).unwrap_or(path).to_path_buf();
        if entry.file_type().is_some_and(|kind| kind.is_dir()) {
            seen_dirs.insert(rel);
            continue;
        }
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        if files.len() >= options.max_files {
            traversal.skipped_after_limit += 1;
            continue;
        }

        let metadata = entry
            .metadata()
            .map_err(|err| RepoError::Walk(err.to_string()))?;
        let is_binary = is_binary_file(path, options.max_bytes_per_snippet).unwrap_or(false);
        let language = detect_language(&rel);
        if let Some(language) = language.as_ref() {
            *languages.entry(language.clone()).or_insert(0) += 1;
        }
        let file = FileEntry {
            path: rel.clone(),
            language: language.clone(),
            size_bytes: metadata.len(),
            modified_unix_secs: modified_unix_secs(&metadata),
            is_binary,
        };
        if is_manifest(&rel) {
            manifests.push(context_document(
                &rel,
                ContextDocumentKind::Manifest,
                language.clone(),
                metadata.len(),
                path,
                options.max_bytes_per_snippet,
                is_binary,
            ));
        } else if is_documentation(&rel) {
            docs.push(context_document(
                &rel,
                ContextDocumentKind::Documentation,
                language.clone(),
                metadata.len(),
                path,
                options.max_bytes_per_snippet,
                is_binary,
            ));
        }
        files.push(file);
    }

    let skipped_dirs = skipped_dirs
        .lock()
        .map(|guard| guard.clone())
        .unwrap_or_default();
    for rel in skipped_dirs {
        if seen_dirs.contains(&rel) {
            continue;
        }
        boundaries.push(ContextBoundary {
            path: rel,
            kind: ContextBoundaryKind::NestedGitRepository,
        });
    }
    traversal.indexed_files = files.len();
    traversal.skipped_nested_contexts = boundaries.len();
    files.sort_by(|left, right| left.path.cmp(&right.path));
    manifests.sort_by(|left, right| left.path.cmp(&right.path));
    docs.sort_by(|left, right| left.path.cmp(&right.path));
    boundaries.sort_by(|left, right| left.path.cmp(&right.path));
    let summary = summarize_root(
        &root,
        files.len(),
        &languages,
        &manifests,
        &docs,
        &boundaries,
    );

    Ok(ContextRootSnapshot {
        id: format!("context-root-{index}"),
        requested_root: requested_root.to_path_buf(),
        root,
        head: head.map(|value| value.trim().to_string()),
        dirty,
        files,
        languages,
        manifests,
        docs,
        boundaries,
        traversal,
        summary,
    })
}

fn should_descend_entry(
    root: &Path,
    entry: &DirEntry,
    include_nested_contexts: bool,
    skipped_dirs: &Arc<Mutex<BTreeSet<PathBuf>>>,
) -> bool {
    let path = entry.path();
    if path == root {
        return true;
    }
    let name = entry.file_name().to_string_lossy();
    if name == ".git" || name == "target" || name == ".tzu" {
        return false;
    }
    if include_nested_contexts || !entry.file_type().is_some_and(|kind| kind.is_dir()) {
        return true;
    }
    if path.join(".git").exists() {
        let rel = path.strip_prefix(root).unwrap_or(path).to_path_buf();
        if let Ok(mut skipped_dirs) = skipped_dirs.lock() {
            skipped_dirs.insert(rel);
        }
        return false;
    }
    true
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

fn context_document(
    path: &Path,
    kind: ContextDocumentKind,
    language: Option<String>,
    size_bytes: u64,
    absolute: &Path,
    max_bytes: usize,
    is_binary: bool,
) -> ContextDocument {
    let snippet = (!is_binary)
        .then(|| {
            read_text_prefix(absolute, max_bytes)
                .ok()
                .map(|(text, _)| text)
        })
        .flatten();
    ContextDocument {
        path: path.to_path_buf(),
        kind,
        language,
        size_bytes,
        snippet,
    }
}

fn is_manifest(path: &Path) -> bool {
    let file = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    matches!(
        file,
        "Cargo.toml"
            | "package.json"
            | "pyproject.toml"
            | "go.mod"
            | "flake.nix"
            | "lakefile.toml"
            | "deno.json"
            | "pom.xml"
    )
}

fn is_documentation(path: &Path) -> bool {
    let file = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(
        file.as_str(),
        "readme.md" | "readme" | "agents.md" | "contributing.md" | "architecture.md"
    ) || path
        .components()
        .any(|component| component.as_os_str() == "docs")
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
        Some("js" | "jsx") => Some("JavaScript".to_string()),
        Some("ts" | "tsx") => Some("TypeScript".to_string()),
        Some("css") => Some("CSS".to_string()),
        Some("html") => Some("HTML".to_string()),
        Some("py") => Some("Python".to_string()),
        Some("go") => Some("Go".to_string()),
        Some("lean") => Some("Lean".to_string()),
        Some("xml") => Some("XML".to_string()),
        _ => None,
    }
}

fn is_binary_file(path: &Path, max_bytes: usize) -> Result<bool, RepoError> {
    let bytes = read_prefix(path, max_bytes)?;
    Ok(matches!(inspect(&bytes), ContentType::BINARY))
}

fn read_text_prefix(path: &Path, max_bytes: usize) -> Result<(String, bool), RepoError> {
    let bytes = read_prefix(path, max_bytes)?;
    let total = fs::metadata(path)?.len();
    let truncated = total > bytes.len() as u64;
    Ok((String::from_utf8_lossy(&bytes).to_string(), truncated))
}

fn read_prefix(path: &Path, max_bytes: usize) -> Result<Vec<u8>, RepoError> {
    let bytes = fs::read(path)?;
    Ok(bytes.into_iter().take(max_bytes.max(1)).collect())
}

fn modified_unix_secs(metadata: &fs::Metadata) -> Option<u64> {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
}

fn normalize_relative_path(path: &Path) -> Option<PathBuf> {
    if path.is_absolute() {
        return None;
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => normalized.push(value),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(normalized)
}

fn summarize_snapshot(roots: &[ContextRootSnapshot]) -> String {
    let file_count = roots.iter().map(|root| root.files.len()).sum::<usize>();
    let root_count = roots.len();
    let boundary_count = roots
        .iter()
        .map(|root| root.boundaries.len())
        .sum::<usize>();
    format!(
        "{root_count} context roots, {file_count} indexed files, {boundary_count} nested boundaries"
    )
}

fn summarize_root(
    root: &Path,
    file_count: usize,
    languages: &BTreeMap<String, usize>,
    manifests: &[ContextDocument],
    docs: &[ContextDocument],
    boundaries: &[ContextBoundary],
) -> String {
    let language_summary = languages
        .iter()
        .map(|(language, count)| format!("{language}={count}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "{}: files={}, languages=[{}], manifests={}, docs={}, nested_boundaries={}",
        root.display(),
        file_count,
        language_summary,
        manifests.len(),
        docs.len(),
        boundaries.len()
    )
}

fn stable_snapshot_id(state_root: &Path, roots: &[ContextRootSnapshot]) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in state_root.display().to_string().bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    for root in roots {
        for byte in root.summary.bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
    }
    format!("context-{hash:016x}")
}

fn context_blocker(
    path: PathBuf,
    reason: ContextBlockerReason,
    description: impl Into<String>,
    snapshot: &ProjectContextSnapshot,
) -> ContextBlocker {
    ContextBlocker {
        path,
        reason,
        description: description.into(),
        producer: "tzu-repo context expansion".to_string(),
        regenerate_command: "tzu plan \"<goal>\" --domain coding --context-root <path>".to_string(),
        validation_command: format!(
            "tzu inspect # validates persisted context for {}",
            snapshot.id
        ),
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

    #[tokio::test]
    async fn context_respects_gitignore_and_detects_documents() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join(".gitignore"), "ignored.log\n").unwrap();
        fs::write(
            temp.path().join("Cargo.toml"),
            "[package]\nname='fixture'\n",
        )
        .unwrap();
        fs::write(temp.path().join("README.md"), "# Fixture\n").unwrap();
        fs::write(temp.path().join("ignored.log"), "hidden").unwrap();
        let output = std::process::Command::new("git")
            .args(["init", "-b", "main"])
            .current_dir(temp.path())
            .output()
            .unwrap();
        assert!(output.status.success());

        let snapshot = inspect_context(temp.path(), InspectOptions::default())
            .await
            .unwrap();
        let root = &snapshot.roots[0];
        let paths = root
            .files
            .iter()
            .map(|file| file.path.as_path())
            .collect::<Vec<_>>();

        assert!(paths.contains(&Path::new("Cargo.toml")));
        assert!(paths.contains(&Path::new("README.md")));
        assert!(!paths.contains(&Path::new("ignored.log")));
        assert_eq!(root.manifests.len(), 1);
        assert_eq!(root.docs.len(), 1);
        assert_eq!(root.languages["TOML"], 1);
        assert_eq!(root.languages["Markdown"], 1);
    }

    #[tokio::test]
    async fn context_skips_nested_repositories_by_default() {
        let temp = tempfile::tempdir().unwrap();
        let nested = temp.path().join("nested");
        fs::create_dir_all(nested.join(".git")).unwrap();
        fs::write(nested.join("lib.rs"), "pub fn nested() {}\n").unwrap();
        fs::write(temp.path().join("main.rs"), "fn main() {}\n").unwrap();

        let snapshot = inspect_context(temp.path(), InspectOptions::default())
            .await
            .unwrap();
        let root = &snapshot.roots[0];
        assert!(
            root.files
                .iter()
                .any(|file| file.path == Path::new("main.rs"))
        );
        assert!(
            !root
                .files
                .iter()
                .any(|file| file.path == Path::new("nested/lib.rs"))
        );
        assert_eq!(root.boundaries.len(), 1);

        let snapshot = inspect_context(
            temp.path(),
            InspectOptions {
                include_nested_contexts: true,
                ..InspectOptions::default()
            },
        )
        .await
        .unwrap();
        let root = &snapshot.roots[0];
        assert!(
            root.files
                .iter()
                .any(|file| file.path == Path::new("nested/lib.rs"))
        );
    }

    #[tokio::test]
    async fn lazy_expansion_blocks_outside_and_stale_paths() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("src.rs");
        fs::write(&source, "fn main() {}\n").unwrap();
        let snapshot = inspect_context(temp.path(), InspectOptions::default())
            .await
            .unwrap();
        fs::write(&source, "fn changed() {}\n").unwrap();

        let root_id = snapshot.roots[0].id.clone();
        let expansion = expand_context_files(
            &snapshot,
            ContextExpansionRequest {
                snapshot_id: snapshot.id.clone(),
                root_id,
                paths: vec![PathBuf::from("../escape.rs"), PathBuf::from("src.rs")],
                max_bytes_per_file: 1024,
            },
        );

        assert!(expansion.files.is_empty());
        assert!(
            expansion
                .blockers
                .iter()
                .any(|blocker| blocker.reason == ContextBlockerReason::OutsideRoot)
        );
        assert!(
            expansion
                .blockers
                .iter()
                .any(|blocker| blocker.reason == ContextBlockerReason::Stale)
        );
    }
}
