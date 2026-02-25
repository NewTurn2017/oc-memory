use oc_core::Storage;
use oc_core::models::{Memory, MemoryMetadata, MemoryType, Priority, SearchQuery};
use oc_embeddings::EmbeddingEngine;
use oc_search::bm25::Bm25Index;
use oc_search::hybrid::HybridSearch;
use oc_search::scoring::Scorer;
use oc_search::vector::VectorIndex;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

/// Shared application state for MCP server.
pub struct McpState {
    pub storage: Arc<Storage>,
    pub search: Mutex<HybridSearch>,
    pub embedder: Option<Arc<EmbeddingEngine>>,
}

// Safety: Storage is behind Arc, HybridSearch behind Mutex
unsafe impl Send for McpState {}
unsafe impl Sync for McpState {}

/// Create an in-memory McpState for testing (no embedding engine, 4-dim vectors).
pub fn test_mcp_state() -> Arc<McpState> {
    let storage = Arc::new(Storage::in_memory().unwrap());
    let vector_index = VectorIndex::new(4);
    let bm25_index = Bm25Index::in_memory().unwrap();
    let scorer = Scorer::default();
    let search = HybridSearch::new(storage.clone(), vector_index, bm25_index, scorer);

    Arc::new(McpState {
        storage,
        search: Mutex::new(search),
        embedder: None,
    })
}

/// Handle a JSON-RPC 2.0 request and return a JSON-RPC response.
pub async fn handle_request(request: &Value, state: &Arc<McpState>) -> Value {
    let method = request["method"].as_str().unwrap_or("");
    let id = request.get("id").cloned();

    let result = match method {
        "initialize" => handle_initialize(),
        "tools/list" => handle_tools_list(),
        "tools/call" => handle_tool_call(request, state).await,
        _ => {
            json!({ "error": { "code": -32601, "message": format!("Method not found: {method}") } })
        }
    };

    let mut response = json!({
        "jsonrpc": "2.0",
        "result": result,
    });

    if let Some(id) = id {
        response["id"] = id;
    }

    response
}

fn handle_initialize() -> Value {
    json!({
        "protocolVersion": "2024-11-05",
        "capabilities": {
            "tools": {}
        },
        "serverInfo": {
            "name": "oc-memory",
            "version": env!("CARGO_PKG_VERSION")
        }
    })
}

fn handle_tools_list() -> Value {
    json!({
        "tools": [
            {
                "name": "memory_search",
                "description": "Search memories using hybrid vector + keyword search. Returns ranked results with relevance scores. Use index_only=true for token-efficient browsing, then memory_get for full content.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Natural language search query" },
                        "limit": { "type": "integer", "description": "Maximum results to return (default: 10)", "default": 10 },
                        "index_only": { "type": "boolean", "description": "If true, return titles/metadata only (saves 90%+ tokens).", "default": false }
                    },
                    "required": ["query"]
                }
            },
            {
                "name": "memory_store",
                "description": "Store a new memory (observation, decision, preference, fact, task, etc.)",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "content": { "type": "string", "description": "Memory content to store" },
                        "title": { "type": "string", "description": "Short title (max 10 words)" },
                        "memory_type": { "type": "string", "enum": ["observation","decision","preference","fact","task","session","bugfix","discovery"], "default": "observation" },
                        "priority": { "type": "string", "enum": ["low","medium","high"], "default": "medium" },
                        "tags": { "type": "array", "items": { "type": "string" } }
                    },
                    "required": ["content", "title"]
                }
            },
            {
                "name": "memory_get",
                "description": "Get full content of specific memories by ID.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "ids": { "type": "array", "items": { "type": "string" }, "description": "Memory IDs to retrieve" }
                    },
                    "required": ["ids"]
                }
            },
            {
                "name": "memory_delete",
                "description": "Delete a memory by ID",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "description": "Memory ID to delete" }
                    },
                    "required": ["id"]
                }
            },
            {
                "name": "memory_stats",
                "description": "Get memory system statistics",
                "inputSchema": { "type": "object", "properties": {} }
            }
        ]
    })
}

async fn handle_tool_call(request: &Value, state: &Arc<McpState>) -> Value {
    let tool_name = request["params"]["name"].as_str().unwrap_or("");
    let arguments = &request["params"]["arguments"];

    match tool_name {
        "memory_search" => tool_memory_search(arguments, state),
        "memory_store" => tool_memory_store(arguments, state),
        "memory_get" => tool_memory_get(arguments, state),
        "memory_delete" => tool_memory_delete(arguments, state),
        "memory_stats" => tool_memory_stats(state),
        _ => json!({
            "content": [{ "type": "text", "text": format!("Unknown tool: {tool_name}") }],
            "isError": true
        }),
    }
}

fn tool_memory_search(args: &Value, state: &Arc<McpState>) -> Value {
    let query_text = args["query"].as_str().unwrap_or("");
    let limit = args["limit"].as_u64().unwrap_or(10) as usize;
    let index_only = args["index_only"].as_bool().unwrap_or(false);

    if query_text.is_empty() {
        return mcp_error("Query cannot be empty");
    }

    let search_query = SearchQuery {
        query: query_text.to_string(),
        limit,
        index_only,
        ..Default::default()
    };

    let query_embedding = state
        .embedder
        .as_ref()
        .and_then(|e| match e.embed(query_text) {
            Ok(emb) => Some(emb),
            Err(err) => {
                tracing::warn!("Embedding failed: {err}, falling back to keyword-only");
                None
            }
        });

    let empty_embedding = vec![0f32; state.embedder.as_ref().map_or(1024, |e| e.dimensions())];
    let embedding_ref = query_embedding.as_deref().unwrap_or(&empty_embedding);

    let search = match state.search.lock() {
        Ok(s) => s,
        Err(e) => return mcp_error(&format!("Search lock error: {e}")),
    };

    match search.search(embedding_ref, &search_query) {
        Ok(results) => {
            if results.is_empty() {
                return mcp_text("No memories found matching your query.");
            }
            let mut output = format!("Found {} memories:\n\n", results.len());
            for (i, result) in results.iter().enumerate() {
                let m = &result.memory;
                let bd = &result.score_breakdown;
                output.push_str(&format!(
                    "{}. **{}** (score: {:.3})\n   ID: {}\n   Type: {} | Priority: {:?} | Tags: {}\n   Scores: sem={:.2} kw={:.2} rec={:.2} imp={:.2}\n",
                    i + 1, m.title, result.score, m.id,
                    m.metadata.memory_type.as_str(), m.metadata.priority,
                    m.metadata.tags.join(", "),
                    bd.semantic, bd.keyword, bd.recency, bd.importance,
                ));
                if !index_only && !m.content.is_empty() {
                    output.push_str(&format!("   Content: {}\n", m.content));
                }
                output.push('\n');
            }
            mcp_text(&output)
        }
        Err(e) => mcp_error(&format!("Search failed: {e}")),
    }
}

fn tool_memory_store(args: &Value, state: &Arc<McpState>) -> Value {
    let content = match args["content"].as_str() {
        Some(c) if !c.is_empty() => c.to_string(),
        _ => return mcp_error("Content is required"),
    };
    let title = match args["title"].as_str() {
        Some(t) if !t.is_empty() => t.to_string(),
        _ => return mcp_error("Title is required"),
    };

    let memory_type = args["memory_type"]
        .as_str()
        .and_then(|s| serde_json::from_str::<MemoryType>(&format!("\"{s}\"")).ok())
        .unwrap_or(MemoryType::Observation);

    let priority = args["priority"]
        .as_str()
        .and_then(|s| serde_json::from_str::<Priority>(&format!("\"{s}\"")).ok())
        .unwrap_or(Priority::Medium);

    let tags: Vec<String> = args["tags"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let embedding = state
        .embedder
        .as_ref()
        .and_then(|e| match e.embed(&content) {
            Ok(emb) => Some(emb),
            Err(err) => {
                tracing::warn!("Embedding failed for new memory: {err}");
                None
            }
        });

    let mut memory = Memory::new(
        content,
        title.clone(),
        MemoryMetadata {
            memory_type,
            priority,
            tags,
            ..Default::default()
        },
    );
    memory.embedding = embedding;

    if let Err(e) = state.storage.insert(&memory) {
        return mcp_error(&format!("Failed to store memory: {e}"));
    }

    if let Ok(mut search) = state.search.lock()
        && let Err(e) = search.index_memory(&memory)
    {
        tracing::warn!("Failed to index memory {}: {e}", memory.id);
    }

    mcp_text(&format!(
        "Memory stored successfully.\nID: {}\nTitle: {}\nType: {}\nEmbedding: {}",
        memory.id,
        title,
        memory_type.as_str(),
        if memory.embedding.is_some() {
            "✓ generated"
        } else {
            "✗ unavailable"
        }
    ))
}

fn tool_memory_get(args: &Value, state: &Arc<McpState>) -> Value {
    let ids: Vec<String> = match args["ids"].as_array() {
        Some(arr) => arr
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect(),
        None => return mcp_error("ids array is required"),
    };

    if ids.is_empty() {
        return mcp_error("ids array cannot be empty");
    }

    match state.storage.get_many(&ids) {
        Ok(memories) => {
            if memories.is_empty() {
                return mcp_text("No memories found with the given IDs.");
            }
            let mut output = String::new();
            for m in &memories {
                output.push_str(&format!(
                    "## {} ({})\n**ID:** {}\n**Type:** {} | **Priority:** {:?}\n**Tags:** {}\n**Created:** {}\n**Content:**\n{}\n\n---\n\n",
                    m.title, m.metadata.memory_type.as_str(), m.id,
                    m.metadata.memory_type.as_str(), m.metadata.priority,
                    m.metadata.tags.join(", "),
                    m.created_at.format("%Y-%m-%d %H:%M"),
                    m.content,
                ));
                let _ = state.storage.touch(&m.id);
            }
            mcp_text(&output)
        }
        Err(e) => mcp_error(&format!("Failed to retrieve memories: {e}")),
    }
}

fn tool_memory_delete(args: &Value, state: &Arc<McpState>) -> Value {
    let id = match args["id"].as_str() {
        Some(id) if !id.is_empty() => id,
        _ => return mcp_error("id is required"),
    };

    if let Ok(mut search) = state.search.lock()
        && let Err(e) = search.remove_memory(id)
    {
        tracing::warn!("Failed to remove from search index: {e}");
    }

    match state.storage.delete(id) {
        Ok(true) => mcp_text(&format!("Memory {} deleted successfully.", id)),
        Ok(false) => mcp_text(&format!("Memory {} not found.", id)),
        Err(e) => mcp_error(&format!("Failed to delete memory: {e}")),
    }
}

fn tool_memory_stats(state: &Arc<McpState>) -> Value {
    let total = state.storage.count().unwrap_or(0);
    let indexed = state.search.lock().map(|s| s.indexed_count()).unwrap_or(0);
    let has_embedder = state.embedder.is_some();

    mcp_text(&format!(
        "Memory System Stats:\n- Total memories: {}\n- Indexed for search: {}\n- Embedding engine: {}\n- Dimensions: 1024\n- Search mode: {}",
        total,
        indexed,
        if has_embedder {
            "✓ active (BGE-m3-ko)"
        } else {
            "✗ not loaded"
        },
        if has_embedder {
            "hybrid (vector + keyword + time decay)"
        } else {
            "keyword-only (BM25)"
        },
    ))
}

/// Format a text response in MCP protocol format.
pub fn mcp_text(text: &str) -> Value {
    json!({
        "content": [{ "type": "text", "text": text }]
    })
}

/// Format an error response in MCP protocol format.
pub fn mcp_error(msg: &str) -> Value {
    json!({
        "content": [{ "type": "text", "text": format!("Error: {msg}") }],
        "isError": true
    })
}
