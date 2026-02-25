use anyhow::Result;
use oc_core::Config;
use oc_embeddings::EmbeddingEngine;
use oc_mcp_server::{McpState, handle_request};
use oc_search::bm25::Bm25Index;
use oc_search::hybrid::HybridSearch;
use oc_search::scoring::Scorer;
use oc_search::vector::VectorIndex;
use serde_json::Value;
use std::io::{self, BufRead, Write};
use std::sync::{Arc, Mutex};

fn init_state(config: &Config) -> Result<Arc<McpState>> {
    let db_path = shellexpand(&config.storage.data_dir);
    std::fs::create_dir_all(&db_path)?;
    let db_file = format!("{}/memories.db", db_path);
    let storage = Arc::new(oc_core::Storage::open(&db_file)?);

    let tantivy_path = format!("{}/tantivy", db_path);
    std::fs::create_dir_all(&tantivy_path)?;

    let vector_index = VectorIndex::new(config.embedding.dimensions);
    let bm25_index = Bm25Index::new(&tantivy_path)?;
    let scorer = Scorer::default();
    let mut search = HybridSearch::new(Arc::clone(&storage), vector_index, bm25_index, scorer);

    // Load existing embeddings into vector index
    let embeddings = storage.all_embeddings()?;
    for (id, embedding) in &embeddings {
        let _ = search
            .vector_index_mut()
            .upsert(id.clone(), embedding.clone());
    }

    // Rebuild BM25 index from existing memories
    let text_data = storage.all_text_data()?;
    let bm25_count = text_data.len();
    for (id, title, content) in &text_data {
        let _ = search.index_memory_text(id, title, content);
    }
    if bm25_count > 0 {
        tracing::info!("Rebuilt BM25 index with {bm25_count} memories");
    }

    let embedder = match init_embedder(config) {
        Ok(engine) => {
            tracing::info!("Embedding engine loaded successfully");
            Some(engine)
        }
        Err(e) => {
            tracing::warn!(
                "Embedding engine not available: {e}. Memory search will use keyword-only mode."
            );
            None
        }
    };

    Ok(Arc::new(McpState {
        storage,
        search: Mutex::new(search),
        embedder,
    }))
}

fn init_embedder(config: &Config) -> Result<Arc<EmbeddingEngine>> {
    let model_path = shellexpand(&config.embedding.model_path);
    let tokenizer_path = shellexpand(&config.embedding.tokenizer_path);

    let engine = EmbeddingEngine::new(
        &model_path,
        &tokenizer_path,
        config.embedding.dimensions,
        config.embedding.max_length,
        config.embedding.num_threads,
    )?;
    Ok(Arc::new(engine))
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_writer(io::stderr)
        .with_env_filter("oc_mcp_server=info")
        .init();

    tracing::info!("oc-memory MCP server starting");

    let config = Config::default();
    let state = init_state(&config)?;

    tracing::info!("oc-memory MCP server ready");

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut stdout = stdout.lock();

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        let request: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                let err_response = serde_json::json!({
                    "jsonrpc": "2.0",
                    "error": { "code": -32700, "message": format!("Parse error: {e}") }
                });
                writeln!(stdout, "{}", serde_json::to_string(&err_response)?)?;
                stdout.flush()?;
                continue;
            }
        };

        let response = handle_request(&request, &state).await;

        let response_str = serde_json::to_string(&response)?;
        writeln!(stdout, "{response_str}")?;
        stdout.flush()?;
    }

    Ok(())
}

fn shellexpand(path: &str) -> String {
    if path.starts_with("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return path.replacen("~", &home.to_string_lossy(), 1);
    }
    path.to_string()
}
