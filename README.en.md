# oc-memory

`oc-memory` is a Rust-native local memory engine for AI assistants. Connect it to OpenClaw/Claude Code via MCP, install the skill guidance from `SKILL.md`, and you get a practical long-term memory workflow out of the box.

![CI](https://img.shields.io/badge/CI-GitHub_Actions-2088FF)
![Rust](https://img.shields.io/badge/Rust-2024-000000)
![Tests](https://img.shields.io/badge/tests-105%20passed-brightgreen)
![License](https://img.shields.io/badge/license-MIT-blue)

Korean README: `README.md`

## Product Goal

- **Skill-first onboarding**: memory behavior improves through skill installation, not manual prompting
- **100% local execution**: no external API key required for inference/search
- **Korean-friendly retrieval**: BGE-m3-ko + lindera BM25
- **Token-efficient retrieval**: progressive disclosure (`index_only` first)

## OpenClaw Integration (recommended)

### OpenClaw cloud installation verification (Ubuntu 24.04)

Validated on a real cloud OpenClaw host:

- `cargo test --workspace` passed (core/search/mcp/server)
- `cargo build --release --workspace` passed
- artifacts: `target/release/oc-memory-mcp` / `target/release/oc-memory-server` (about 95MB each)

### 0) Cloud prerequisites (required)

```bash
sudo apt update
sudo apt install -y build-essential pkg-config libssl-dev clang cmake curl

curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"

rustc --version
cargo --version
```

### 0) One-liner setup (copy/paste)

For fast bootstrap in OpenClaw environments:

```bash
source "$HOME/.cargo/env" && \
mkdir -p ~/.config/oc-memory && \
cargo build --release --workspace && \
python3 - <<'PY'
import json
from pathlib import Path

cfg = Path.home() / ".claude" / "config.json"
cfg.parent.mkdir(parents=True, exist_ok=True)
data = {}
if cfg.exists():
    try:
        data = json.loads(cfg.read_text(encoding="utf-8"))
    except Exception:
        data = {}
data.setdefault("mcpServers", {})["memory"] = {
    "command": str(Path.cwd() / "target/release/oc-memory-mcp"),
    "args": []
}
cfg.write_text(json.dumps(data, ensure_ascii=False, indent=2), encoding="utf-8")
print(f"Updated MCP config: {cfg}")
PY
```

### 1) Build binaries

```bash
source "$HOME/.cargo/env"
cargo build --release --workspace
```

Generated binaries:

- `target/release/oc-memory-mcp`
- `target/release/oc-memory-server`

### 2) Register MCP server

Add this to `~/.claude/config.json` (or equivalent OpenClaw MCP config):

```json
{
  "mcpServers": {
    "memory": {
      "command": "/absolute/path/to/target/release/oc-memory-mcp",
      "args": []
    }
  }
}
```

### 3) Install/register the skill

Register `SKILL.md` in your OpenClaw skill system so the assistant consistently follows:

- context lookup at session start
- automatic storage of key decisions/preferences/fixes
- token-saving read path (`index_only -> memory_get`)

## 60-second demo

This is the exact flow that shows the "skill-install-only" improvement.

![oc-memory demo flow](docs/assets/demo-flow.svg)

```bash
# 1) Start a new session after MCP + skill registration
# 2) Ask: "What did we decide last time?"
# 3) memory_search(index_only=true) -> memory_get only for relevant IDs
# 4) Ask: "Store this as a decision" -> memory_store(decision/high)
```

## Auto Recall (E2E)

To retrieve prior context automatically in new sessions:

```bash
bash scripts/oc-memory-auto-recall.sh "recent work" 3
```

Use the Auto Recall section in `SKILL.md` for OpenClaw always-on memory policy and full verification flow.

## MCP Tools

- `memory_search`: hybrid retrieval (vector + BM25)
- `memory_store`: persist memory entries
- `memory_get`: fetch full content by ID
- `memory_delete`: remove memory entries
- `memory_stats`: memory/index status

## Architecture

```text
crates/
├── core/          # models, SQLite storage, config
├── embeddings/    # BGE-m3-ko ONNX Runtime
├── search/        # usearch HNSW + BM25 + hybrid scoring
├── observer/      # file watcher
├── mcp-server/    # MCP JSON-RPC stdio
└── server/        # REST API (axum)
```

Scoring model:

```text
score = semantic(0.6) + keyword(0.15) + recency(0.15) + importance(0.10)
recency = exp(-ln(2)/30 * days_since_access)
```

## Verified Status

Current workspace verification:

- `cargo test --workspace`: **105 passed, 0 failed**
- `cargo build --workspace`: pass
- `cargo clippy --workspace -- -D warnings -A clippy::arc-with-non-send-sync`: pass

## CI/CD

Workflow file: `.github/workflows/ci.yml`

- Triggers: `push(main)`, `pull_request(main)`, `workflow_dispatch`, weekly `schedule`
- `check` job: build/test/clippy/fmt + `cargo audit`
- `release-build` job: release build and artifact upload on `main`

## Troubleshooting (OpenClaw cloud install)

### 1) `rustc: command not found`

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"
```

### 2) `linker cc not found`

```bash
sudo apt update
sudo apt install -y build-essential pkg-config libssl-dev clang cmake
```

### 3) `lindera-ko-dic` broken dictionary URL (`invalid gzip header`, `NoSuchBucket`)

CI already applies an automatic patch in `.github/workflows/ci.yml`.
For manual local/cloud builds, you may still need this fallback:

```bash
cargo fetch
BUILDRS=$(find ~/.cargo/registry/src -path '*/lindera-ko-dic-*/build.rs' | head -1)
sed -i 's|https://lindera.s3.ap-northeast-1.amazonaws.com/mecab-ko-dic-2.1.1-20180720.tar.gz|https://bitbucket.org/eunjeon/mecab-ko-dic/downloads/mecab-ko-dic-2.1.1-20180720.tar.gz|' "$BUILDRS"
```

Important: this edits Cargo registry cache files and can break again after cleanup/fresh environments.
For a permanent solution, keep a repository-level dependency patch strategy (e.g. `[patch.crates-io]` or upgrade path) under version control.

### 4) systemd migration example (observer -> Rust server)

`/etc/systemd/system/oc-memory.service`:

```ini
[Unit]
Description=oc-memory REST Server
After=network.target

[Service]
Type=simple
User=root
WorkingDirectory=/root/oc-memory
ExecStart=/root/oc-memory/target/release/oc-memory-server
Restart=always
RestartSec=5
Environment=RUST_LOG=info
Environment=LD_LIBRARY_PATH=/root/oc-memory/.venv-model/lib/python3.12/site-packages/onnxruntime/capi

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload
sudo systemctl enable oc-memory.service
sudo systemctl restart oc-memory.service
curl -sS http://127.0.0.1:6342/api/v1/stats
```

Verification target:
- `has_embedder: true`
- `search_mode: "hybrid"`

## Local Development

```bash
source "$HOME/.cargo/env"

cargo fmt --all
cargo clippy --workspace -- -D warnings -A clippy::arc-with-non-send-sync
cargo test --workspace
cargo build --release --workspace
```

Model setup:

```bash
# recommended (venv + prebuilt ONNX download)
bash scripts/setup_model.sh

# or
# direct prebuilt ONNX download
python3 scripts/download_model.py

# optional: local export/quantization path
python3 scripts/download_model.py --convert
```

## Documentation

- Skill behavior and trigger guide: `SKILL.md`
- Project engineering constraints: `CLAUDE.md`

## License

MIT
