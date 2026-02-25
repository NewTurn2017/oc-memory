# oc-memory — Project Instructions

## Overview
Rust-native AI long-term memory engine. Local embeddings (BGE-m3-ko ONNX INT8) + hybrid search (usearch HNSW + lindera BM25 + time decay). MCP server for Claude Code integration.

## Build & Test

```bash
# IMPORTANT: Use rustup Rust, not Homebrew
# Linux/Cloud (recommended)
source "$HOME/.cargo/env"

# macOS fallback (if rustup shell init is not loaded)
export PATH="$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$HOME/.cargo/bin:$PATH"

# Ubuntu 24.04 prerequisites (first install)
sudo apt update
sudo apt install -y build-essential pkg-config libssl-dev clang cmake curl

# Build
cargo build --workspace

# Test (no model needed — tests use fake embeddings)
cargo test --workspace

# Release build
cargo build --release --workspace
```

### lindera-ko-dic S3 Fix
The `lindera-ko-dic` crate's build script downloads from a defunct S3 bucket. After `cargo clean` or fresh clone, patch:

```bash
cargo fetch
BUILDRS=$(find ~/.cargo/registry/src -path '*/lindera-ko-dic-*/build.rs' | head -1)
# Linux
sed -i 's|https://lindera.s3.ap-northeast-1.amazonaws.com/mecab-ko-dic-2.1.1-20180720.tar.gz|https://bitbucket.org/eunjeon/mecab-ko-dic/downloads/mecab-ko-dic-2.1.1-20180720.tar.gz|' "$BUILDRS"

# macOS
sed -i '' 's|https://lindera.s3.ap-northeast-1.amazonaws.com/mecab-ko-dic-2.1.1-20180720.tar.gz|https://bitbucket.org/eunjeon/mecab-ko-dic/downloads/mecab-ko-dic-2.1.1-20180720.tar.gz|' "$BUILDRS"
```

This is a registry-cache level workaround and can regress after environment reset.
Prefer a repository-level permanent strategy (`[patch.crates-io]` or dependency upgrade) for long-term stability.

## Architecture

```
crates/
├── core/          # Data models (Memory, SearchQuery, etc.), SQLite storage, config
├── embeddings/    # BGE-m3-ko ONNX Runtime engine (ort 2.0, Mutex<Session>)
├── search/        # Vector (usearch HNSW) + BM25 (tantivy + lindera ko-dic) + hybrid RRF
├── observer/      # File watcher (notify crate)
├── mcp-server/    # MCP JSON-RPC stdio server (5 tools)
└── server/        # REST API (axum, port 6342)
```

## Key Patterns

### Send/Sync for AppState
Both `mcp-server` and `server` use `Mutex<T>` wrappers with `unsafe impl Send/Sync`:
```rust
struct AppState {
    storage: Mutex<Storage>,
    search: Mutex<HybridSearch>,
    embedder: Option<Arc<EmbeddingEngine>>,
}
unsafe impl Send for AppState {}
unsafe impl Sync for AppState {}
```

### Vector Index (usearch)
- `crates/search/src/vector.rs` — HNSW index via `usearch` crate
- Uses `u64` keys internally; bidirectional map to String UUIDs
- Cosine metric returns distance; convert: `similarity = 1.0 - distance`

### BM25 Index (lindera + tantivy)
- `crates/search/src/bm25.rs` — Korean morphological tokenizer via `lindera-tantivy` with ko-dic
- Enables matching inflected Korean forms (e.g., "한국어" matches "한국어로")

### Scoring Formula
```
score = semantic(0.6) + keyword(0.15) + recency(0.15) + importance(0.10)
recency = exp(-ln(2)/30 × days_since_access)  // 30-day half-life
```

## Constraints
- **Zero external API keys**: All inference runs locally (ONNX Runtime)
- **Target environment**: 4 CPU / 8GB RAM / Ubuntu
- **Rust edition 2024**, minimum Rust 1.85 (ort 2.0 requirement)
- Never suppress type errors with `as any` / `#[allow]` — fix root causes
- Keep the public API of `VectorIndex` stable — `hybrid.rs`, `mcp-server`, and `server` depend on it

## Testing
- Unit tests live alongside source in `mod tests` blocks
- Integration tests: `crates/search/tests/integration_test.rs`
- Tests run WITHOUT the embedding model (use fake 4-dim vectors)
- Currently 19 tests total (3 core + 10 search + 6 integration)

## Model Setup
```bash
# Download and convert BGE-m3-ko to ONNX INT8
python3 scripts/download_model.py
# Or use the shell wrapper
bash scripts/setup_model.sh
```
Model files go in `models/` (gitignored).

## MCP Tools
| Tool | Description |
|------|-------------|
| `memory_search` | Hybrid search (vector + keyword) |
| `memory_store` | Store a memory |
| `memory_get` | Get full memory by ID |
| `memory_delete` | Delete a memory |
| `memory_stats` | System statistics |
