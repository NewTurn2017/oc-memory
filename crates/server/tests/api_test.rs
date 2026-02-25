use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use oc_server::{ApiResponse, StatsResponse, StoreResponse, build_router, test_app_state};
use serde_json::Value;
use tower::ServiceExt;

/// Helper: send a request to the app and return (status, body_bytes).
async fn send(method: &str, uri: &str, body: Option<Value>) -> (StatusCode, bytes::Bytes) {
    let state = test_app_state();
    let app = build_router(state);

    let mut builder = Request::builder().method(method).uri(uri);
    let req = if let Some(json) = body {
        builder = builder.header("content-type", "application/json");
        builder
            .body(Body::from(serde_json::to_vec(&json).unwrap()))
            .unwrap()
    } else {
        builder.body(Body::empty()).unwrap()
    };

    let response = app.oneshot(req).await.unwrap();
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, body)
}

/// Helper: send a request to an app with persistent state.
async fn send_with_state(
    app: axum::Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, bytes::Bytes) {
    let mut builder = Request::builder().method(method).uri(uri);
    let req = if let Some(json) = body {
        builder = builder.header("content-type", "application/json");
        builder
            .body(Body::from(serde_json::to_vec(&json).unwrap()))
            .unwrap()
    } else {
        builder.body(Body::empty()).unwrap()
    };

    let response = app.oneshot(req).await.unwrap();
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, body)
}

// ─── Health ────────────────────────────────────────────────

#[tokio::test]
async fn health_returns_ok() {
    let (status, body) = send("GET", "/health", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(&body[..], b"ok");
}

// ─── Store ─────────────────────────────────────────────────

#[tokio::test]
async fn store_memory_returns_201() {
    let payload = serde_json::json!({
        "content": "Rust는 메모리 안전한 시스템 프로그래밍 언어입니다.",
        "title": "Rust 소개",
        "memory_type": "observation",
        "priority": "high",
        "tags": ["rust", "programming"]
    });
    let (status, body) = send("POST", "/api/v1/memories", Some(payload)).await;
    assert_eq!(status, StatusCode::CREATED);

    let resp: ApiResponse<StoreResponse> = serde_json::from_slice(&body).unwrap();
    assert!(resp.success);
    let data = resp.data.unwrap();
    assert_eq!(data.title, "Rust 소개");
    assert!(!data.id.is_empty());
    // No embedder in test mode
    assert!(!data.has_embedding);
}

#[tokio::test]
async fn store_memory_with_defaults() {
    let payload = serde_json::json!({
        "content": "Default test",
        "title": "Default"
    });
    let (status, body) = send("POST", "/api/v1/memories", Some(payload)).await;
    assert_eq!(status, StatusCode::CREATED);

    let resp: ApiResponse<StoreResponse> = serde_json::from_slice(&body).unwrap();
    assert!(resp.success);
}

#[tokio::test]
async fn store_with_invalid_json_returns_error() {
    let state = test_app_state();
    let app = build_router(state);

    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/memories")
        .header("content-type", "application/json")
        .body(Body::from(b"not json".to_vec()))
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    // axum returns 400 Bad Request for JSON parse errors, 422 for deserialization errors
    let status = response.status();
    assert!(
        status == StatusCode::BAD_REQUEST || status == StatusCode::UNPROCESSABLE_ENTITY,
        "Expected 400 or 422, got {status}",
    );
}

// ─── Search ────────────────────────────────────────────────

#[tokio::test]
async fn search_empty_db_returns_empty() {
    let payload = serde_json::json!({
        "query": "rust",
        "limit": 5
    });
    let (status, body) = send("POST", "/api/v1/search", Some(payload)).await;
    assert_eq!(status, StatusCode::OK);

    let resp: ApiResponse<Vec<Value>> = serde_json::from_slice(&body).unwrap();
    assert!(resp.success);
    assert!(resp.data.unwrap().is_empty());
}

#[tokio::test]
async fn store_then_search_finds_memory() {
    let state = test_app_state();
    let app = build_router(state);

    // Store
    let store_req = Request::builder()
        .method("POST")
        .uri("/api/v1/memories")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({
                "content": "한국어 형태소 분석은 NLP에서 중요합니다.",
                "title": "한국어 NLP",
                "tags": ["korean", "nlp"]
            }))
            .unwrap(),
        ))
        .unwrap();

    let resp = app.clone().oneshot(store_req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    // Search — BM25 keyword match
    let search_req = Request::builder()
        .method("POST")
        .uri("/api/v1/search")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({
                "query": "한국어 형태소",
                "limit": 10
            }))
            .unwrap(),
        ))
        .unwrap();

    let resp = app.oneshot(search_req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let search_resp: ApiResponse<Vec<Value>> = serde_json::from_slice(&body).unwrap();
    assert!(search_resp.success);
    let results = search_resp.data.unwrap();
    assert!(!results.is_empty(), "BM25 should find the stored memory");
}

// ─── Get / Delete ──────────────────────────────────────────

#[tokio::test]
async fn get_nonexistent_returns_404() {
    let (status, body) = send("GET", "/api/v1/memories/nonexistent-id", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let resp: ApiResponse<Value> = serde_json::from_slice(&body).unwrap();
    assert!(!resp.success);
    assert!(resp.error.unwrap().contains("Not found"));
}

#[tokio::test]
async fn delete_nonexistent_returns_404() {
    let (status, body) = send("DELETE", "/api/v1/memories/nonexistent-id", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let resp: ApiResponse<Value> = serde_json::from_slice(&body).unwrap();
    assert!(!resp.success);
}

#[tokio::test]
async fn store_get_delete_lifecycle() {
    let state = test_app_state();
    let app = build_router(state);

    // 1. Store
    let (status, body) = send_with_state(
        app.clone(),
        "POST",
        "/api/v1/memories",
        Some(serde_json::json!({
            "content": "Lifecycle test content",
            "title": "Lifecycle"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let store_resp: ApiResponse<StoreResponse> = serde_json::from_slice(&body).unwrap();
    let id = store_resp.data.unwrap().id;

    // 2. Get
    let (status, body) =
        send_with_state(app.clone(), "GET", &format!("/api/v1/memories/{id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    let get_resp: ApiResponse<Value> = serde_json::from_slice(&body).unwrap();
    assert!(get_resp.success);
    let memory_data = get_resp.data.unwrap();
    assert_eq!(memory_data["title"], "Lifecycle");
    assert_eq!(memory_data["content"], "Lifecycle test content");

    // 3. Delete
    let (status, body) = send_with_state(
        app.clone(),
        "DELETE",
        &format!("/api/v1/memories/{id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let del_resp: ApiResponse<Value> = serde_json::from_slice(&body).unwrap();
    assert!(del_resp.success);

    // 4. Verify deleted
    let (status, _) = send_with_state(app, "GET", &format!("/api/v1/memories/{id}"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ─── Stats ─────────────────────────────────────────────────

#[tokio::test]
async fn stats_empty_db() {
    let (status, body) = send("GET", "/api/v1/stats", None).await;
    assert_eq!(status, StatusCode::OK);

    let resp: ApiResponse<StatsResponse> = serde_json::from_slice(&body).unwrap();
    assert!(resp.success);
    let stats = resp.data.unwrap();
    assert_eq!(stats.total_memories, 0);
    assert_eq!(stats.indexed_count, 0);
    assert!(!stats.has_embedder);
    assert_eq!(stats.search_mode, "keyword-only");
}

#[tokio::test]
async fn stats_after_store() {
    let state = test_app_state();
    let app = build_router(state.clone());

    // Store one memory
    let (status, _) = send_with_state(
        app.clone(),
        "POST",
        "/api/v1/memories",
        Some(serde_json::json!({
            "content": "Stats test",
            "title": "Stats"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // Verify insert persisted via direct state access
    let count = state.storage.lock().unwrap().count().unwrap();
    assert_eq!(count, 1, "Direct storage count should be 1 after insert");

    // Check stats via API
    let (status, body) = send_with_state(app, "GET", "/api/v1/stats", None).await;
    assert_eq!(status, StatusCode::OK);

    let resp: ApiResponse<StatsResponse> = serde_json::from_slice(&body).unwrap();
    let stats = resp.data.unwrap();
    assert_eq!(stats.total_memories, 1);
    // indexed_count reflects vector index entries; without embedder, this stays 0
    // BM25 indexing still works (verified by search tests)
    assert_eq!(stats.indexed_count, 0);
}

// ─── Edge cases ────────────────────────────────────────────

#[tokio::test]
async fn store_unicode_content() {
    let payload = serde_json::json!({
        "content": "日本語テスト 🎉 中文测试 한국어 테스트",
        "title": "Unicode Mix",
        "tags": ["unicode", "multilingual"]
    });
    let (status, body) = send("POST", "/api/v1/memories", Some(payload)).await;
    assert_eq!(status, StatusCode::CREATED);

    let resp: ApiResponse<StoreResponse> = serde_json::from_slice(&body).unwrap();
    assert!(resp.success);
    assert_eq!(resp.data.unwrap().title, "Unicode Mix");
}

#[tokio::test]
async fn search_with_index_only() {
    let state = test_app_state();
    let app = build_router(state);

    // Store
    let (status, _) = send_with_state(
        app.clone(),
        "POST",
        "/api/v1/memories",
        Some(serde_json::json!({
            "content": "Index only test content that should not appear in results",
            "title": "Index Only Test"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // Search with index_only
    let (status, body) = send_with_state(
        app,
        "POST",
        "/api/v1/search",
        Some(serde_json::json!({
            "query": "index only test",
            "index_only": true,
            "limit": 10
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let resp: ApiResponse<Vec<Value>> = serde_json::from_slice(&body).unwrap();
    assert!(resp.success);
}

#[tokio::test]
async fn unknown_route_returns_404() {
    let (status, _) = send("GET", "/api/v1/nonexistent", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
