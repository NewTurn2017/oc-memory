use anyhow::Result;
use oc_core::Config;
use oc_embeddings::EmbeddingEngine;
use oc_search::bm25::Bm25Index;
use oc_search::hybrid::HybridSearch;
use oc_search::scoring::Scorer;
use oc_search::vector::VectorIndex;
use oc_server::{AppState, SharedState, build_router};
use std::sync::{Arc, Mutex};

fn init_app(config: &Config) -> Result<AppState> {
    let db_path = shellexpand(&config.storage.data_dir);
    std::fs::create_dir_all(&db_path)?;
    let db_file = format!("{}/memories.db", db_path);
    let storage = oc_core::Storage::open(&db_file)?;

    let tantivy_path = format!("{}/tantivy", db_path);
    std::fs::create_dir_all(&tantivy_path)?;

    // We need a separate storage instance for HybridSearch since it expects Arc<Storage>
    let search_storage = Arc::new(oc_core::Storage::open(&db_file)?);

    let vector_index = VectorIndex::new(config.embedding.dimensions);
    let bm25_index = Bm25Index::new(&tantivy_path)?;
    let scorer = Scorer::default();
    let mut search = HybridSearch::new(search_storage.clone(), vector_index, bm25_index, scorer);

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
            tracing::info!("Embedding engine loaded");
            Some(engine)
        }
        Err(e) => {
            tracing::warn!("Embedding engine not available: {e}");
            None
        }
    };

    Ok(AppState {
        storage: Mutex::new(storage),
        search: Mutex::new(search),
        embedder,
    })
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
        .with_env_filter("oc_server=info,tower_http=debug")
        .init();

    let config = Config::default();
    let state: SharedState = Arc::new(init_app(&config)?);

    let addr = format!("{}:{}", config.server.host, config.server.port);
    tracing::info!("oc-memory REST server starting on {addr}");

    let app = build_router(state).layer(tower_http::trace::TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

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
