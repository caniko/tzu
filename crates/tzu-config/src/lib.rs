use std::collections::BTreeMap;
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
    pub include_nested_contexts: bool,
    #[serde(default)]
    pub gui: GuiSettings,
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
    config_path_from_env(&std::env::vars().collect())
}

pub fn load_config() -> Result<TzuConfig, ConfigError> {
    load_config_with_env(&std::env::vars().collect())
}

pub fn load_config_with_env(env: &BTreeMap<String, String>) -> Result<TzuConfig, ConfigError> {
    let path = config_path_from_env(env);
    let mut config = match path {
        Some(path) if path.exists() => parse_config_file(&path)?,
        _ => TzuConfig::default(),
    };
    apply_env_overrides(&mut config, env)?;
    Ok(config)
}

pub fn discover_projects(config: &TzuConfig) -> Result<Vec<DiscoveredProject>, ConfigError> {
    let Some(root) = config.projects_directory.as_ref() else {
        return Ok(Vec::new());
    };
    discover_projects_in(root)
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
        projects.push(DiscoveredProject {
            name,
            path,
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
    }
    if let Some(value) = env
        .get("TZU_INCLUDE_NESTED_CONTEXTS")
        .filter(|value| !value.is_empty())
    {
        config.include_nested_contexts = parse_bool("TZU_INCLUDE_NESTED_CONTEXTS", value)?;
    }
    Ok(())
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
    fn reads_xdg_toml_config() {
        let temp = tempfile::tempdir().unwrap();
        let config_home = temp.path().join("config");
        let config_dir = config_home.join("tzu");
        fs::create_dir_all(&config_dir).unwrap();
        fs::write(
            config_dir.join("config.toml"),
            r#"
projects_directory = "/projects"
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
        assert!(config.include_nested_contexts);
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
}
