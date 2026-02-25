use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Memory entry — the atomic unit of stored knowledge
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub id: String,
    pub content: String,
    pub title: String,
    pub metadata: MemoryMetadata,
    pub embedding: Option<Vec<f32>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub accessed_at: DateTime<Utc>,
    pub access_count: u32,
}

impl Memory {
    pub fn new(content: String, title: String, metadata: MemoryMetadata) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            content,
            title,
            metadata,
            embedding: None,
            created_at: now,
            updated_at: now,
            accessed_at: now,
            access_count: 0,
        }
    }

    /// Token count estimate (rough: 1 token ≈ 3.5 chars for Korean)
    pub fn estimated_tokens(&self) -> usize {
        (self.content.len() as f64 / 3.5).ceil() as usize
    }
}

/// Metadata attached to each memory
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryMetadata {
    pub memory_type: MemoryType,
    pub priority: Priority,
    pub source: Option<String>,
    pub tags: Vec<String>,
    pub concepts: Vec<String>,
    pub files: Vec<String>,
}

impl Default for MemoryMetadata {
    fn default() -> Self {
        Self {
            memory_type: MemoryType::Observation,
            priority: Priority::Medium,
            source: None,
            tags: Vec::new(),
            concepts: Vec::new(),
            files: Vec::new(),
        }
    }
}

/// Type of memory entry
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryType {
    Observation,
    Decision,
    Preference,
    Fact,
    Task,
    Session,
    Bugfix,
    Discovery,
}

impl MemoryType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Observation => "observation",
            Self::Decision => "decision",
            Self::Preference => "preference",
            Self::Fact => "fact",
            Self::Task => "task",
            Self::Session => "session",
            Self::Bugfix => "bugfix",
            Self::Discovery => "discovery",
        }
    }
}

/// Priority level for ranking
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    Low = 0,
    Medium = 1,
    High = 2,
}

impl Priority {
    /// Weight factor for search scoring
    pub fn weight(&self) -> f32 {
        match self {
            Self::Low => 0.4,
            Self::Medium => 0.7,
            Self::High => 1.0,
        }
    }
}

/// Search query parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchQuery {
    pub query: String,
    pub limit: usize,
    pub memory_type: Option<MemoryType>,
    pub priority: Option<Priority>,
    pub tags: Option<Vec<String>>,
    /// If true, return index only (titles + metadata, minimal tokens)
    pub index_only: bool,
}

impl Default for SearchQuery {
    fn default() -> Self {
        Self {
            query: String::new(),
            limit: 10,
            memory_type: None,
            priority: None,
            tags: None,
            index_only: false,
        }
    }
}

/// Search result with scoring
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub memory: Memory,
    pub score: f32,
    /// Breakdown: vector similarity, BM25 score, recency, importance
    pub score_breakdown: ScoreBreakdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoreBreakdown {
    pub semantic: f32,
    pub keyword: f32,
    pub recency: f32,
    pub importance: f32,
}
