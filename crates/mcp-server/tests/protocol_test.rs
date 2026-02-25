use oc_mcp_server::{handle_request, test_mcp_state};
use serde_json::{Value, json};

// ─── Helpers ───────────────────────────────────────────────

fn jsonrpc(method: &str, params: Option<Value>) -> Value {
    let mut req = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method
    });
    if let Some(p) = params {
        req["params"] = p;
    }
    req
}

fn extract_text(response: &Value) -> String {
    response["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or("")
        .to_string()
}

fn is_error_response(response: &Value) -> bool {
    response["result"]["isError"].as_bool().unwrap_or(false)
}

// ─── initialize ────────────────────────────────────────────

#[tokio::test]
async fn initialize_returns_protocol_version() {
    let state = test_mcp_state();
    let req = jsonrpc("initialize", None);
    let resp = handle_request(&req, &state).await;

    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 1);
    assert_eq!(resp["result"]["protocolVersion"], "2024-11-05");
    assert!(resp["result"]["serverInfo"]["name"].is_string());
    assert!(resp["result"]["capabilities"]["tools"].is_object());
}

// ─── tools/list ────────────────────────────────────────────

#[tokio::test]
async fn tools_list_returns_five_tools() {
    let state = test_mcp_state();
    let req = jsonrpc("tools/list", None);
    let resp = handle_request(&req, &state).await;

    let tools = resp["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 5);

    let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
    assert!(names.contains(&"memory_search"));
    assert!(names.contains(&"memory_store"));
    assert!(names.contains(&"memory_get"));
    assert!(names.contains(&"memory_delete"));
    assert!(names.contains(&"memory_stats"));
}

#[tokio::test]
async fn tools_have_input_schemas() {
    let state = test_mcp_state();
    let req = jsonrpc("tools/list", None);
    let resp = handle_request(&req, &state).await;

    let tools = resp["result"]["tools"].as_array().unwrap();
    for tool in tools {
        assert!(
            tool["inputSchema"].is_object(),
            "Tool {} missing inputSchema",
            tool["name"]
        );
        assert_eq!(tool["inputSchema"]["type"], "object");
    }
}

// ─── memory_store ──────────────────────────────────────────

#[tokio::test]
async fn store_and_verify() {
    let state = test_mcp_state();

    let req = jsonrpc(
        "tools/call",
        Some(json!({
            "name": "memory_store",
            "arguments": {
                "content": "Rust는 시스템 프로그래밍 언어입니다.",
                "title": "Rust 소개",
                "memory_type": "observation",
                "priority": "high",
                "tags": ["rust", "language"]
            }
        })),
    );

    let resp = handle_request(&req, &state).await;
    let text = extract_text(&resp);
    assert!(!is_error_response(&resp));
    assert!(text.contains("Memory stored successfully"));
    assert!(text.contains("Rust 소개"));
    assert!(text.contains("ID:"));
}

#[tokio::test]
async fn store_missing_content_returns_error() {
    let state = test_mcp_state();

    let req = jsonrpc(
        "tools/call",
        Some(json!({
            "name": "memory_store",
            "arguments": {
                "title": "Missing Content"
            }
        })),
    );

    let resp = handle_request(&req, &state).await;
    assert!(is_error_response(&resp));
    assert!(extract_text(&resp).contains("Content is required"));
}

#[tokio::test]
async fn store_missing_title_returns_error() {
    let state = test_mcp_state();

    let req = jsonrpc(
        "tools/call",
        Some(json!({
            "name": "memory_store",
            "arguments": {
                "content": "Has content but no title"
            }
        })),
    );

    let resp = handle_request(&req, &state).await;
    assert!(is_error_response(&resp));
    assert!(extract_text(&resp).contains("Title is required"));
}

#[tokio::test]
async fn store_with_defaults() {
    let state = test_mcp_state();

    let req = jsonrpc(
        "tools/call",
        Some(json!({
            "name": "memory_store",
            "arguments": {
                "content": "Default test",
                "title": "Defaults"
            }
        })),
    );

    let resp = handle_request(&req, &state).await;
    assert!(!is_error_response(&resp));
    assert!(extract_text(&resp).contains("Memory stored successfully"));
}

// ─── memory_search ─────────────────────────────────────────

#[tokio::test]
async fn search_empty_db() {
    let state = test_mcp_state();

    let req = jsonrpc(
        "tools/call",
        Some(json!({
            "name": "memory_search",
            "arguments": { "query": "rust" }
        })),
    );

    let resp = handle_request(&req, &state).await;
    assert!(!is_error_response(&resp));
    assert!(extract_text(&resp).contains("No memories found"));
}

#[tokio::test]
async fn search_empty_query_returns_error() {
    let state = test_mcp_state();

    let req = jsonrpc(
        "tools/call",
        Some(json!({
            "name": "memory_search",
            "arguments": { "query": "" }
        })),
    );

    let resp = handle_request(&req, &state).await;
    assert!(is_error_response(&resp));
    assert!(extract_text(&resp).contains("Query cannot be empty"));
}

#[tokio::test]
async fn store_then_search_bm25() {
    let state = test_mcp_state();

    // Store
    let store_req = jsonrpc(
        "tools/call",
        Some(json!({
            "name": "memory_store",
            "arguments": {
                "content": "한국어 형태소 분석은 자연어처리에서 매우 중요합니다.",
                "title": "한국어 NLP 중요성"
            }
        })),
    );
    let resp = handle_request(&store_req, &state).await;
    assert!(!is_error_response(&resp));

    // Search via BM25
    let search_req = jsonrpc(
        "tools/call",
        Some(json!({
            "name": "memory_search",
            "arguments": { "query": "한국어 형태소", "limit": 5 }
        })),
    );
    let resp = handle_request(&search_req, &state).await;
    let text = extract_text(&resp);
    assert!(!is_error_response(&resp));
    assert!(text.contains("Found"), "Should find stored memory via BM25");
    assert!(text.contains("한국어 NLP"));
}

// ─── memory_get ────────────────────────────────────────────

#[tokio::test]
async fn get_nonexistent_ids() {
    let state = test_mcp_state();

    let req = jsonrpc(
        "tools/call",
        Some(json!({
            "name": "memory_get",
            "arguments": { "ids": ["nonexistent-id"] }
        })),
    );

    let resp = handle_request(&req, &state).await;
    assert!(!is_error_response(&resp));
    assert!(extract_text(&resp).contains("No memories found"));
}

#[tokio::test]
async fn get_empty_ids_returns_error() {
    let state = test_mcp_state();

    let req = jsonrpc(
        "tools/call",
        Some(json!({
            "name": "memory_get",
            "arguments": { "ids": [] }
        })),
    );

    let resp = handle_request(&req, &state).await;
    assert!(is_error_response(&resp));
    assert!(extract_text(&resp).contains("empty"));
}

#[tokio::test]
async fn store_then_get_by_id() {
    let state = test_mcp_state();

    // Store
    let store_req = jsonrpc(
        "tools/call",
        Some(json!({
            "name": "memory_store",
            "arguments": {
                "content": "Get test content",
                "title": "Get Test"
            }
        })),
    );
    let resp = handle_request(&store_req, &state).await;
    let text = extract_text(&resp);

    // Extract ID from response
    let id = text
        .lines()
        .find(|l| l.starts_with("ID:"))
        .map(|l| l.trim_start_matches("ID:").trim())
        .expect("Should have ID in response");

    // Get by ID
    let get_req = jsonrpc(
        "tools/call",
        Some(json!({
            "name": "memory_get",
            "arguments": { "ids": [id] }
        })),
    );
    let resp = handle_request(&get_req, &state).await;
    let text = extract_text(&resp);
    assert!(!is_error_response(&resp));
    assert!(text.contains("Get Test"));
    assert!(text.contains("Get test content"));
}

// ─── memory_delete ─────────────────────────────────────────

#[tokio::test]
async fn delete_nonexistent() {
    let state = test_mcp_state();

    let req = jsonrpc(
        "tools/call",
        Some(json!({
            "name": "memory_delete",
            "arguments": { "id": "does-not-exist" }
        })),
    );

    let resp = handle_request(&req, &state).await;
    assert!(!is_error_response(&resp));
    assert!(extract_text(&resp).contains("not found"));
}

#[tokio::test]
async fn store_then_delete() {
    let state = test_mcp_state();

    // Store
    let store_req = jsonrpc(
        "tools/call",
        Some(json!({
            "name": "memory_store",
            "arguments": {
                "content": "Delete me",
                "title": "To Delete"
            }
        })),
    );
    let resp = handle_request(&store_req, &state).await;
    let text = extract_text(&resp);
    let id = text
        .lines()
        .find(|l| l.starts_with("ID:"))
        .map(|l| l.trim_start_matches("ID:").trim())
        .unwrap();

    // Delete
    let del_req = jsonrpc(
        "tools/call",
        Some(json!({
            "name": "memory_delete",
            "arguments": { "id": id }
        })),
    );
    let resp = handle_request(&del_req, &state).await;
    assert!(!is_error_response(&resp));
    assert!(extract_text(&resp).contains("deleted successfully"));

    // Verify deleted
    let get_req = jsonrpc(
        "tools/call",
        Some(json!({
            "name": "memory_get",
            "arguments": { "ids": [id] }
        })),
    );
    let resp = handle_request(&get_req, &state).await;
    assert!(extract_text(&resp).contains("No memories found"));
}

#[tokio::test]
async fn delete_missing_id_returns_error() {
    let state = test_mcp_state();

    let req = jsonrpc(
        "tools/call",
        Some(json!({
            "name": "memory_delete",
            "arguments": {}
        })),
    );

    let resp = handle_request(&req, &state).await;
    assert!(is_error_response(&resp));
    assert!(extract_text(&resp).contains("id is required"));
}

// ─── memory_stats ──────────────────────────────────────────

#[tokio::test]
async fn stats_empty() {
    let state = test_mcp_state();

    let req = jsonrpc(
        "tools/call",
        Some(json!({
            "name": "memory_stats",
            "arguments": {}
        })),
    );

    let resp = handle_request(&req, &state).await;
    let text = extract_text(&resp);
    assert!(!is_error_response(&resp));
    assert!(text.contains("Total memories: 0"));
    assert!(text.contains("keyword-only"));
}

#[tokio::test]
async fn stats_after_store() {
    let state = test_mcp_state();

    // Store
    let store_req = jsonrpc(
        "tools/call",
        Some(json!({
            "name": "memory_store",
            "arguments": {
                "content": "Stats test",
                "title": "Stats"
            }
        })),
    );
    handle_request(&store_req, &state).await;

    // Stats
    let req = jsonrpc(
        "tools/call",
        Some(json!({
            "name": "memory_stats",
            "arguments": {}
        })),
    );
    let resp = handle_request(&req, &state).await;
    let text = extract_text(&resp);
    assert!(text.contains("Total memories: 1"));
}

// ─── Unknown method / tool ─────────────────────────────────

#[tokio::test]
async fn unknown_method() {
    let state = test_mcp_state();
    let req = jsonrpc("nonexistent/method", None);
    let resp = handle_request(&req, &state).await;

    assert!(resp["result"]["error"].is_object());
    assert_eq!(resp["result"]["error"]["code"], -32601);
}

#[tokio::test]
async fn unknown_tool_call() {
    let state = test_mcp_state();

    let req = jsonrpc(
        "tools/call",
        Some(json!({
            "name": "nonexistent_tool",
            "arguments": {}
        })),
    );

    let resp = handle_request(&req, &state).await;
    assert!(is_error_response(&resp));
    assert!(extract_text(&resp).contains("Unknown tool"));
}

// ─── JSON-RPC ID propagation ───────────────────────────────

#[tokio::test]
async fn response_includes_request_id() {
    let state = test_mcp_state();

    let req = json!({
        "jsonrpc": "2.0",
        "id": 42,
        "method": "initialize"
    });

    let resp = handle_request(&req, &state).await;
    assert_eq!(resp["id"], 42);
}

#[tokio::test]
async fn response_with_string_id() {
    let state = test_mcp_state();

    let req = json!({
        "jsonrpc": "2.0",
        "id": "req-abc-123",
        "method": "tools/list"
    });

    let resp = handle_request(&req, &state).await;
    assert_eq!(resp["id"], "req-abc-123");
}

// ─── Full lifecycle ────────────────────────────────────────

#[tokio::test]
async fn full_lifecycle_store_search_get_delete_stats() {
    let state = test_mcp_state();

    // 1. Stats: 0 memories
    let resp = handle_request(
        &jsonrpc(
            "tools/call",
            Some(json!({"name": "memory_stats", "arguments": {}})),
        ),
        &state,
    )
    .await;
    assert!(extract_text(&resp).contains("Total memories: 0"));

    // 2. Store
    let resp = handle_request(
        &jsonrpc(
            "tools/call",
            Some(json!({
                "name": "memory_store",
                "arguments": {
                    "content": "Lifecycle test: Rust memory engine",
                    "title": "Lifecycle Test",
                    "tags": ["lifecycle"]
                }
            })),
        ),
        &state,
    )
    .await;
    let text = extract_text(&resp);
    assert!(text.contains("Memory stored"));
    let id = text
        .lines()
        .find(|l| l.starts_with("ID:"))
        .map(|l| l.trim_start_matches("ID:").trim().to_string())
        .unwrap();

    // 3. Stats: 1 memory
    let resp = handle_request(
        &jsonrpc(
            "tools/call",
            Some(json!({"name": "memory_stats", "arguments": {}})),
        ),
        &state,
    )
    .await;
    assert!(extract_text(&resp).contains("Total memories: 1"));

    // 4. Search by keyword
    let resp = handle_request(
        &jsonrpc(
            "tools/call",
            Some(json!({
                "name": "memory_search",
                "arguments": { "query": "Lifecycle Test memory engine", "limit": 5 }
            })),
        ),
        &state,
    )
    .await;
    assert!(extract_text(&resp).contains("Lifecycle Test"));

    // 5. Get by ID
    let resp = handle_request(
        &jsonrpc(
            "tools/call",
            Some(json!({
                "name": "memory_get",
                "arguments": { "ids": [&id] }
            })),
        ),
        &state,
    )
    .await;
    let text = extract_text(&resp);
    assert!(text.contains("Lifecycle Test"));
    assert!(text.contains("Lifecycle test: Rust memory engine"));

    // 6. Delete
    let resp = handle_request(
        &jsonrpc(
            "tools/call",
            Some(json!({
                "name": "memory_delete",
                "arguments": { "id": &id }
            })),
        ),
        &state,
    )
    .await;
    assert!(extract_text(&resp).contains("deleted successfully"));

    // 7. Stats: 0 memories again
    let resp = handle_request(
        &jsonrpc(
            "tools/call",
            Some(json!({"name": "memory_stats", "arguments": {}})),
        ),
        &state,
    )
    .await;
    assert!(extract_text(&resp).contains("Total memories: 0"));
}
