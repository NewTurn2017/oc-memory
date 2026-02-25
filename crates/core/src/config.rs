use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// Main configuration for oc-memory engine
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    pub storage: StorageConfig,
    pub embedding: EmbeddingConfig,
    pub search: SearchConfig,
    pub observer: ObserverConfig,
    pub server: ServerConfig,
}

impl Config {
    /// Load configuration from a TOML file
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self> {
        let content =
            std::fs::read_to_string(path.as_ref()).map_err(|e| Error::Config(e.to_string()))?;
        toml::from_str(&content).map_err(|e| Error::Config(e.to_string()))
    }

    /// Load from default path (~/.config/oc-memory/config.toml)
    pub fn load_default() -> Result<Self> {
        let home = dirs_path();
        let config_path = home.join("config.toml");

        if config_path.exists() {
            Self::from_file(config_path)
        } else {
            Ok(Self::default())
        }
    }

    /// Data directory path
    pub fn data_dir(&self) -> PathBuf {
        let path = shellexpand(&self.storage.data_dir);
        PathBuf::from(path)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    /// Data directory for SQLite DB and indices
    pub data_dir: String,
    /// Maximum hot memory entries
    pub max_hot_memories: usize,
    /// Hot memory TTL in days
    pub hot_ttl_days: u32,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            data_dir: "~/.local/share/oc-memory".to_string(),
            max_hot_memories: 10_000,
            hot_ttl_days: 90,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingConfig {
    /// Path to ONNX model file
    pub model_path: String,
    /// Path to tokenizer.json
    pub tokenizer_path: String,
    /// Vector dimensions (1024 for BGE-m3-ko)
    pub dimensions: usize,
    /// Max sequence length
    pub max_length: usize,
    /// Number of threads for ONNX Runtime
    pub num_threads: usize,
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            model_path: "~/.local/share/oc-memory/models/bge-m3-ko-int8.onnx".to_string(),
            tokenizer_path: "~/.local/share/oc-memory/models/tokenizer.json".to_string(),
            dimensions: 1024,
            max_length: 8192,
            num_threads: 4,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchConfig {
    /// Weight for semantic (vector) similarity
    pub semantic_weight: f32,
    /// Weight for BM25 keyword score
    pub keyword_weight: f32,
    /// Weight for recency score
    pub recency_weight: f32,
    /// Weight for importance/priority score
    pub importance_weight: f32,
    /// Recency half-life in days (exponential decay)
    pub recency_half_life_days: f32,
    /// Default number of results
    pub default_limit: usize,
    /// HNSW ef_search parameter
    pub ef_search: usize,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            semantic_weight: 0.6,
            keyword_weight: 0.15,
            recency_weight: 0.15,
            importance_weight: 0.10,
            recency_half_life_days: 30.0,
            default_limit: 10,
            ef_search: 100,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObserverConfig {
    /// Directories to watch for file changes
    pub watch_dirs: Vec<String>,
    /// Watch subdirectories recursively
    pub recursive: bool,
    /// File extensions to monitor
    pub extensions: Vec<String>,
}

impl Default for ObserverConfig {
    fn default() -> Self {
        Self {
            watch_dirs: Vec::new(),
            recursive: true,
            extensions: vec!["md".to_string(), "markdown".to_string(), "txt".to_string()],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// REST API host
    pub host: String,
    /// REST API port
    pub port: u16,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 6342,
        }
    }
}

/// Expand ~ to home directory
fn shellexpand(path: &str) -> String {
    if path.starts_with("~/")
        && let Some(home) = home_dir()
    {
        return path.replacen("~", &home.to_string_lossy(), 1);
    }
    path.to_string()
}

/// Config directory: ~/.config/oc-memory/
fn dirs_path() -> PathBuf {
    let mut path = home_dir().unwrap_or_else(|| PathBuf::from("."));
    path.push(".config");
    path.push("oc-memory");
    path
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}
