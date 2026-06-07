use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

const KNOWN_PROJECT_MARKERS: &[&str] = &[
    ".git",
    "Cargo.toml",
    "package.json",
    "flake.nix",
    "pyproject.toml",
    "go.mod",
    "lakefile.toml",
];

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("required config file is missing: `{path}`")]
    MissingRequiredConfig { path: PathBuf },
    #[error("config parse error in `{path}`: {message}")]
    Parse { path: PathBuf, message: String },
    #[error("invalid boolean for `{key}`: `{value}`")]
    InvalidBool { key: String, value: String },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TzuConfig {
    #[serde(default)]
    pub projects_directory: Option<PathBuf>,
    #[serde(default)]
    pub projects_directories: Vec<PathBuf>,
    #[serde(default)]
    pub include_nested_contexts: bool,
    #[serde(default)]
    pub gui: GuiSettings,
}

impl TzuConfig {
    #[must_use]
    pub fn project_discovery_roots(&self) -> Vec<PathBuf> {
        let mut roots = self.projects_directories.clone();
        if let Some(root) = self.projects_directory.clone() {
            roots.push(root);
        }
        deduplicate_paths(roots)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuiSettings {
    #[serde(default = "default_gui_host")]
    pub host: String,
    #[serde(default = "default_gui_port")]
    pub port: u16,
    #[serde(default)]
    pub enable_service: bool,
}

impl Default for GuiSettings {
    fn default() -> Self {
        Self {
            host: default_gui_host(),
            port: default_gui_port(),
            enable_service: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredProject {
    pub name: String,
    pub path: PathBuf,
    pub markers: Vec<String>,
}

#[must_use]
pub fn default_config_path() -> Option<PathBuf> {
    config_path_with_env(&std::env::vars().collect())
}

#[must_use]
pub fn config_path_with_env(env: &BTreeMap<String, String>) -> Option<PathBuf> {
    config_path_from_env(env)
}

pub fn load_config() -> Result<TzuConfig, ConfigError> {
    load_config_with_env(&std::env::vars().collect())
}

pub fn load_config_with_env(env: &BTreeMap<String, String>) -> Result<TzuConfig, ConfigError> {
    let path = config_path_from_env(env);
    let require_config = env
        .get("TZU_REQUIRE_CONFIG")
        .filter(|value| !value.is_empty())
        .map(|value| parse_bool("TZU_REQUIRE_CONFIG", value))
        .transpose()?
        .unwrap_or(false);
    let mut config = match path {
        Some(path) if path.exists() => parse_config_file(&path)?,
        Some(path) if require_config => return Err(ConfigError::MissingRequiredConfig { path }),
        _ => TzuConfig::default(),
    };
    apply_env_overrides(&mut config, env)?;
    Ok(config)
}

pub fn discover_projects(config: &TzuConfig) -> Result<Vec<DiscoveredProject>, ConfigError> {
    discover_projects_in_roots(config.project_discovery_roots())
}

pub fn discover_projects_in_roots(
    roots: impl IntoIterator<Item = PathBuf>,
) -> Result<Vec<DiscoveredProject>, ConfigError> {
    let mut seen = BTreeSet::new();
    let mut projects = Vec::new();
    for root in roots {
        for project in discover_projects_in(&root)? {
            let canonical = project
                .path
                .canonicalize()
                .unwrap_or_else(|_| project.path.clone());
            if seen.insert(canonical.clone()) {
                projects.push(DiscoveredProject {
                    path: canonical,
                    ..project
                });
            }
        }
    }
    projects.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.path.cmp(&right.path))
    });
    Ok(projects)
}

pub fn discover_projects_in(root: &Path) -> Result<Vec<DiscoveredProject>, ConfigError> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut projects = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let markers = project_markers(&path);
        if markers.is_empty() {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_string();
        let canonical = path.canonicalize().unwrap_or(path);
        projects.push(DiscoveredProject {
            name,
            path: canonical,
            markers,
        });
    }
    projects.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.path.cmp(&right.path))
    });
    Ok(projects)
}

#[must_use]
pub fn project_markers(path: &Path) -> Vec<String> {
    KNOWN_PROJECT_MARKERS
        .iter()
        .filter(|marker| path.join(marker).exists())
        .map(|marker| (*marker).to_string())
        .collect()
}

fn parse_config_file(path: &Path) -> Result<TzuConfig, ConfigError> {
    let text = fs::read_to_string(path)?;
    toml::from_str(&text).map_err(|err| ConfigError::Parse {
        path: path.to_path_buf(),
        message: err.to_string(),
    })
}

fn apply_env_overrides(
    config: &mut TzuConfig,
    env: &BTreeMap<String, String>,
) -> Result<(), ConfigError> {
    if let Some(value) = env
        .get("TZU_PROJECTS_DIR")
        .filter(|value| !value.is_empty())
    {
        config.projects_directory = Some(PathBuf::from(value));
        config.projects_directories.clear();
    }
    if let Some(value) = env
        .get("TZU_PROJECTS_DIRS")
        .filter(|value| !value.is_empty())
    {
        config.projects_directories = env::split_paths(value).collect();
        config.projects_directory = None;
    }
    if let Some(value) = env
        .get("TZU_INCLUDE_NESTED_CONTEXTS")
        .filter(|value| !value.is_empty())
    {
        config.include_nested_contexts = parse_bool("TZU_INCLUDE_NESTED_CONTEXTS", value)?;
    }
    Ok(())
}

fn deduplicate_paths(paths: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    let mut seen = BTreeSet::new();
    let mut deduplicated = Vec::new();
    for path in paths {
        let key = path.canonicalize().unwrap_or_else(|_| path.clone());
        if seen.insert(key) {
            deduplicated.push(path);
        }
    }
    deduplicated
}

fn parse_bool(key: &str, value: &str) -> Result<bool, ConfigError> {
    match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(ConfigError::InvalidBool {
            key: key.to_string(),
            value: value.to_string(),
        }),
    }
}

fn config_path_from_env(env: &BTreeMap<String, String>) -> Option<PathBuf> {
    env.get("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            env.get("HOME")
                .filter(|value| !value.is_empty())
                .map(|home| PathBuf::from(home).join(".config"))
        })
        .map(|config_home| config_home.join("tzu/config.toml"))
}

fn default_gui_host() -> String {
    "127.0.0.1".to_string()
}

const fn default_gui_port() -> u16 {
    7070
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_config_yields_default_config() {
        let temp = tempfile::tempdir().unwrap();
        let env = BTreeMap::from([(
            "XDG_CONFIG_HOME".to_string(),
            temp.path().join("config").display().to_string(),
        )]);

        let config = load_config_with_env(&env).unwrap();

        assert_eq!(config, TzuConfig::default());
    }

    #[test]
    fn missing_required_config_fails_with_xdg_path() {
        let temp = tempfile::tempdir().unwrap();
        let config_home = temp.path().join("config");
        let env = BTreeMap::from([
            (
                "XDG_CONFIG_HOME".to_string(),
                config_home.display().to_string(),
            ),
            ("TZU_REQUIRE_CONFIG".to_string(), "true".to_string()),
        ]);

        let error = load_config_with_env(&env).unwrap_err();

        match error {
            ConfigError::MissingRequiredConfig { path } => {
                assert_eq!(path, config_home.join("tzu/config.toml"));
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn existing_required_xdg_config_loads_normally() {
        let temp = tempfile::tempdir().unwrap();
        let config_home = temp.path().join("config");
        let config_dir = config_home.join("tzu");
        fs::create_dir_all(&config_dir).unwrap();
        fs::write(
            config_dir.join("config.toml"),
            r#"
projects_directory = "/projects"
"#,
        )
        .unwrap();
        let env = BTreeMap::from([
            (
                "XDG_CONFIG_HOME".to_string(),
                config_home.display().to_string(),
            ),
            ("TZU_REQUIRE_CONFIG".to_string(), "true".to_string()),
        ]);

        let config = load_config_with_env(&env).unwrap();

        assert_eq!(config.projects_directory, Some(PathBuf::from("/projects")));
    }

    #[test]
    fn invalid_require_config_bool_fails_clearly() {
        let env = BTreeMap::from([("TZU_REQUIRE_CONFIG".to_string(), "maybe".to_string())]);

        let error = load_config_with_env(&env).unwrap_err();

        match error {
            ConfigError::InvalidBool { key, value } => {
                assert_eq!(key, "TZU_REQUIRE_CONFIG");
                assert_eq!(value, "maybe");
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn reads_xdg_toml_config() {
        let temp = tempfile::tempdir().unwrap();
        let config_home = temp.path().join("config");
        let config_dir = config_home.join("tzu");
        fs::create_dir_all(&config_dir).unwrap();
        fs::write(
            config_dir.join("config.toml"),
            r#"
projects_directory = "/projects"
projects_directories = ["/extra-projects", "/projects"]
include_nested_contexts = true

[gui]
host = "127.0.0.1"
port = 9090
enable_service = true
"#,
        )
        .unwrap();
        let env = BTreeMap::from([(
            "XDG_CONFIG_HOME".to_string(),
            config_home.display().to_string(),
        )]);

        let config = load_config_with_env(&env).unwrap();

        assert_eq!(config.projects_directory, Some(PathBuf::from("/projects")));
        assert_eq!(
            config.projects_directories,
            vec![PathBuf::from("/extra-projects"), PathBuf::from("/projects")]
        );
        assert_eq!(
            config.project_discovery_roots(),
            vec![PathBuf::from("/extra-projects"), PathBuf::from("/projects")]
        );
        assert!(config.include_nested_contexts);
        assert_eq!(config.gui.port, 9090);
        assert!(config.gui.enable_service);
    }

    #[test]
    fn env_overrides_config_values() {
        let temp = tempfile::tempdir().unwrap();
        let config_home = temp.path().join("config");
        let config_dir = config_home.join("tzu");
        fs::create_dir_all(&config_dir).unwrap();
        fs::write(
            config_dir.join("config.toml"),
            r#"
projects_directory = "/config-projects"
projects_directories = ["/config-extra-projects"]
include_nested_contexts = false
"#,
        )
        .unwrap();
        let env = BTreeMap::from([
            (
                "XDG_CONFIG_HOME".to_string(),
                config_home.display().to_string(),
            ),
            ("TZU_PROJECTS_DIR".to_string(), "/env-projects".to_string()),
            (
                "TZU_INCLUDE_NESTED_CONTEXTS".to_string(),
                "true".to_string(),
            ),
        ]);

        let config = load_config_with_env(&env).unwrap();

        assert_eq!(
            config.projects_directory,
            Some(PathBuf::from("/env-projects"))
        );
        assert!(config.projects_directories.is_empty());
        assert!(config.include_nested_contexts);
    }

    #[test]
    fn plural_env_override_takes_precedence_over_single_env_override() {
        let env_roots = env::join_paths(["/env-a", "/env-b"]).unwrap();
        let env = BTreeMap::from([
            ("TZU_PROJECTS_DIR".to_string(), "/env-single".to_string()),
            (
                "TZU_PROJECTS_DIRS".to_string(),
                env_roots.to_string_lossy().to_string(),
            ),
        ]);

        let config = load_config_with_env(&env).unwrap();

        assert_eq!(config.projects_directory, None);
        assert_eq!(
            config.projects_directories,
            vec![PathBuf::from("/env-a"), PathBuf::from("/env-b")]
        );
    }

    #[test]
    fn env_overrides_with_multiple_project_roots() {
        let temp = tempfile::tempdir().unwrap();
        let config_home = temp.path().join("config");
        let config_dir = config_home.join("tzu");
        fs::create_dir_all(&config_dir).unwrap();
        fs::write(
            config_dir.join("config.toml"),
            r#"
projects_directory = "/config-projects"
projects_directories = ["/also-config-projects"]
"#,
        )
        .unwrap();
        let roots = env::join_paths(["/env-a", "/env-b"]).unwrap();
        let env = BTreeMap::from([
            (
                "XDG_CONFIG_HOME".to_string(),
                config_home.display().to_string(),
            ),
            (
                "TZU_PROJECTS_DIRS".to_string(),
                roots.to_string_lossy().to_string(),
            ),
        ]);

        let config = load_config_with_env(&env).unwrap();

        assert_eq!(config.projects_directory, None);
        assert_eq!(
            config.projects_directories,
            vec![PathBuf::from("/env-a"), PathBuf::from("/env-b")]
        );
    }

    #[test]
    fn discovers_direct_child_projects_only() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        let manifest = temp.path().join("manifest");
        let plain = temp.path().join("plain");
        let nested = plain.join("nested");
        fs::create_dir_all(repo.join(".git")).unwrap();
        fs::create_dir_all(&manifest).unwrap();
        fs::write(manifest.join("Cargo.toml"), "[package]\nname='fixture'\n").unwrap();
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("flake.nix"), "{}").unwrap();

        let projects = discover_projects_in(temp.path()).unwrap();

        assert_eq!(
            projects
                .iter()
                .map(|project| project.name.as_str())
                .collect::<Vec<_>>(),
            vec!["manifest", "repo"]
        );
        assert!(projects[0].markers.contains(&"Cargo.toml".to_string()));
        assert!(projects[1].markers.contains(&".git".to_string()));
    }

    #[test]
    fn discovers_projects_across_multiple_roots_and_deduplicates() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        let alpha = first.join("alpha");
        let beta = second.join("beta");
        let plain = first.join("plain");
        fs::create_dir_all(alpha.join(".git")).unwrap();
        fs::create_dir_all(&beta).unwrap();
        fs::write(beta.join("flake.nix"), "{}").unwrap();
        fs::create_dir_all(plain.join("nested")).unwrap();
        fs::write(
            plain.join("nested").join("Cargo.toml"),
            "[package]\nname='nested'\n",
        )
        .unwrap();

        let projects =
            discover_projects_in_roots([first.clone(), second, first.canonicalize().unwrap()])
                .unwrap();

        assert_eq!(
            projects
                .iter()
                .map(|project| project.name.as_str())
                .collect::<Vec<_>>(),
            vec!["alpha", "beta"]
        );
        assert_eq!(projects[0].path, alpha.canonicalize().unwrap());
        assert_eq!(projects[1].path, beta.canonicalize().unwrap());
    }
}
