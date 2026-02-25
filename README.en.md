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

### 0) One-liner setup (copy/paste)

For fast bootstrap in OpenClaw environments:

```bash
export PATH="$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$HOME/.cargo/bin:$PATH" && \
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
export PATH="$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$HOME/.cargo/bin:$PATH"
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

## Local Development

```bash
export PATH="$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$HOME/.cargo/bin:$PATH"

cargo fmt --all
cargo clippy --workspace -- -D warnings -A clippy::arc-with-non-send-sync
cargo test --workspace
cargo build --release --workspace
```

Model setup:

```bash
bash scripts/setup_model.sh
# or
python3 scripts/download_model.py
```

## Documentation

- Skill behavior and trigger guide: `SKILL.md`
- Project engineering constraints: `CLAUDE.md`

## License

MIT
