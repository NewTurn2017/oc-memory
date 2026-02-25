use axum::{
    Router,
    extract::{Json, Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use oc_core::Storage;
use oc_core::models::{Memory, MemoryMetadata, MemoryType, Priority, SearchQuery, SearchResult};
use oc_embeddings::EmbeddingEngine;
use oc_search::bm25::Bm25Index;
use oc_search::hybrid::HybridSearch;
use oc_search::scoring::Scorer;
use oc_search::vector::VectorIndex;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

/// Shared application state for REST server
pub struct AppState {
    pub storage: Mutex<Storage>,
    pub search: Mutex<HybridSearch>,
    pub embedder: Option<Arc<EmbeddingEngine>>,
}

// Safety: We manually ensure thread-safety via Mutex wrappers
unsafe impl Send for AppState {}
unsafe impl Sync for AppState {}

pub type SharedState = Arc<AppState>;

/// Create an in-memory AppState for testing (no embedding engine).
///
/// Uses a shared temp file so that `state.storage` and `HybridSearch.storage`
/// read from the same database (in-memory SQLite creates isolated DBs per connection).
pub fn test_app_state() -> SharedState {
    let tmp = std::env::temp_dir().join(format!("oc_test_{}.db", uuid::Uuid::new_v4()));
    let db_path = tmp.to_str().unwrap();
    let storage = Storage::open(db_path).unwrap();
    let search_storage = Arc::new(Storage::open(db_path).unwrap());
    let vector_index = VectorIndex::new(4);
    let bm25_index = Bm25Index::in_memory().unwrap();
    let scorer = Scorer::default();
    let search = HybridSearch::new(search_storage, vector_index, bm25_index, scorer);

    Arc::new(AppState {
        storage: Mutex::new(storage),
        search: Mutex::new(search),
        embedder: None,
    })
}

/// Build the axum Router with all routes.
pub fn build_router(state: SharedState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/api/v1/search", post(api_search))
        .route("/api/v1/memories", post(api_store))
        .route("/api/v1/memories/{id}", get(api_get).delete(api_delete))
        .route("/api/v1/stats", get(api_stats))
        .with_state(state)
}

async fn health() -> &'static str {
    "ok"
}

// --- Request / Response types ---

#[derive(Deserialize)]
pub struct SearchRequest {
    pub query: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub index_only: bool,
}

fn default_limit() -> usize {
    10
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ApiResponse<T: Serialize> {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl<T: Serialize> ApiResponse<T> {
    pub fn ok(data: T) -> Self {
        Self {
            success: true,
            data: Some(data),
            error: None,
        }
    }
    pub fn err(msg: impl Into<String>) -> Self {
        Self {
            success: false,
            data: None,
            error: Some(msg.into()),
        }
    }
}

// --- Handlers ---

async fn api_search(
    State(state): State<SharedState>,
    Json(req): Json<SearchRequest>,
) -> Json<ApiResponse<Vec<SearchResult>>> {
    let search_query = SearchQuery {
        query: req.query.clone(),
        limit: req.limit,
        index_only: req.index_only,
        ..Default::default()
    };

    let query_embedding = state
        .embedder
        .as_ref()
        .and_then(|e| e.embed(&req.query).ok());
    let empty = vec![0f32; state.embedder.as_ref().map_or(1024, |e| e.dimensions())];
    let emb = query_embedding.as_deref().unwrap_or(&empty);

    let search = match state.search.lock() {
        Ok(s) => s,
        Err(e) => return Json(ApiResponse::err(format!("Lock error: {e}"))),
    };

    match search.search(emb, &search_query) {
        Ok(results) => Json(ApiResponse::ok(results)),
        Err(e) => Json(ApiResponse::err(format!("Search failed: {e}"))),
    }
}

#[derive(Deserialize)]
pub struct StoreRequest {
    pub content: String,
    pub title: String,
    #[serde(default = "default_type")]
    pub memory_type: String,
    #[serde(default = "default_pri")]
    pub priority: String,
    #[serde(default)]
    pub tags: Vec<String>,
}
fn default_type() -> String {
    "observation".to_string()
}
fn default_pri() -> String {
    "medium".to_string()
}

#[derive(Serialize, Deserialize, Debug)]
pub struct StoreResponse {
    pub id: String,
    pub title: String,
    pub has_embedding: bool,
}

async fn api_store(
    State(state): State<SharedState>,
    Json(req): Json<StoreRequest>,
) -> (StatusCode, Json<ApiResponse<StoreResponse>>) {
    let memory_type = serde_json::from_str::<MemoryType>(&format!("\"{}\"", req.memory_type))
        .unwrap_or(MemoryType::Observation);
    let priority = serde_json::from_str::<Priority>(&format!("\"{}\"", req.priority))
        .unwrap_or(Priority::Medium);

    let embedding = state
        .embedder
        .as_ref()
        .and_then(|e| e.embed(&req.content).ok());

    let mut memory = Memory::new(
        req.content,
        req.title.clone(),
        MemoryMetadata {
            memory_type,
            priority,
            tags: req.tags,
            ..Default::default()
        },
    );
    memory.embedding = embedding.clone();

    // Store in SQLite
    {
        let storage = match state.storage.lock() {
            Ok(s) => s,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiResponse::err(format!("Lock: {e}"))),
                );
            }
        };
        if let Err(e) = storage.insert(&memory) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse::err(format!("Storage: {e}"))),
            );
        }
    }

    // Index in search
    if let Ok(mut search) = state.search.lock() {
        let _ = search.index_memory(&memory);
    }

    (
        StatusCode::CREATED,
        Json(ApiResponse::ok(StoreResponse {
            id: memory.id,
            title: req.title,
            has_embedding: embedding.is_some(),
        })),
    )
}

async fn api_get(State(state): State<SharedState>, Path(id): Path<String>) -> impl IntoResponse {
    let storage = match state.storage.lock() {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse::<()>::err(format!("Lock: {e}"))),
            )
                .into_response();
        }
    };

    match storage.get(&id) {
        Ok(Some(mut memory)) => {
            let _ = storage.touch(&id);
            memory.embedding = None;
            drop(storage);
            Json(ApiResponse::ok(memory)).into_response()
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<()>::err("Not found")),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse::<()>::err(format!("{e}"))),
        )
            .into_response(),
    }
}

async fn api_delete(State(state): State<SharedState>, Path(id): Path<String>) -> impl IntoResponse {
    if let Ok(mut search) = state.search.lock() {
        let _ = search.remove_memory(&id);
    }

    let storage = match state.storage.lock() {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse::<()>::err(format!("Lock: {e}"))),
            )
                .into_response();
        }
    };

    match storage.delete(&id) {
        Ok(true) => Json(ApiResponse::ok("deleted")).into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<()>::err("Not found")),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse::<()>::err(format!("{e}"))),
        )
            .into_response(),
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct StatsResponse {
    pub total_memories: usize,
    pub indexed_count: usize,
    pub has_embedder: bool,
    pub search_mode: String,
}

async fn api_stats(State(state): State<SharedState>) -> Json<ApiResponse<StatsResponse>> {
    let total = state
        .storage
        .lock()
        .map(|s| s.count().unwrap_or(0))
        .unwrap_or(0);
    let indexed = state.search.lock().map(|s| s.indexed_count()).unwrap_or(0);
    let has_embedder = state.embedder.is_some();

    Json(ApiResponse::ok(StatsResponse {
        total_memories: total,
        indexed_count: indexed,
        has_embedder,
        search_mode: if has_embedder {
            "hybrid".to_string()
        } else {
            "keyword-only".to_string()
        },
    }))
}
