# oc-memory

Rust 기반 로컬 AI 메모리 엔진입니다. OpenClaw/Claude Code에 MCP로 연결하고, `SKILL.md` 기반 스킬을 설치하면 세션마다 기억을 검색/저장하는 워크플로우를 바로 사용할 수 있습니다.

![CI](https://img.shields.io/badge/CI-GitHub_Actions-2088FF)
![Rust](https://img.shields.io/badge/Rust-2024-000000)
![Tests](https://img.shields.io/badge/tests-105%20passed-brightgreen)
![License](https://img.shields.io/badge/license-MIT-blue)

English README: `README.en.md`

## 핵심 목표

- **Skill-first UX**: 스킬 설치만으로 메모리 검색/저장 패턴을 표준화
- **100% Local**: 외부 API 키 없이 임베딩 + 검색 로컬 실행
- **한국어 강점**: BGE-m3-ko + lindera BM25 조합
- **Token Efficiency**: `index_only` 기반 progressive disclosure

## OpenClaw 통합 (권장 경로)

### 0) One-liner 설치 (복붙)

OpenClaw 환경에서 빠르게 시작할 때:

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

### 1) 바이너리 빌드

```bash
export PATH="$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$HOME/.cargo/bin:$PATH"
cargo build --release --workspace
```

생성 바이너리:

- `target/release/oc-memory-mcp`
- `target/release/oc-memory-server`

### 2) MCP 서버 등록

`~/.claude/config.json`(또는 OpenClaw가 참조하는 동일 형식 설정)에 등록:

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

### 3) 스킬 설치/등록

이 저장소의 `SKILL.md`를 OpenClaw 스킬 시스템에 등록하면, 아래 패턴이 자동화됩니다.

- 세션 시작 시 선호도/최근 컨텍스트 검색
- 중요 결정/선호/버그 해결책 저장
- `index_only -> memory_get`의 토큰 절약형 조회 흐름

## 1분 데모

아래 흐름 그대로면 “스킬 설치만으로 메모리 품질 향상”이 동작합니다.

![oc-memory demo flow](docs/assets/demo-flow.svg)

```bash
# 1) 통합 후 세션 시작
# 2) "지난번 결정 뭐였지?" 질문
# 3) memory_search(index_only=true) -> 필요한 항목만 memory_get
# 4) "이 결정 저장해줘" -> memory_store(decision/high)
```

## 제공 MCP 도구

- `memory_search`: 하이브리드 검색 (vector + BM25)
- `memory_store`: 메모리 저장
- `memory_get`: ID 기반 전체 조회
- `memory_delete`: 메모리 삭제
- `memory_stats`: 시스템 통계

## 아키텍처

```text
crates/
├── core/          # 모델, SQLite storage, config
├── embeddings/    # BGE-m3-ko ONNX Runtime
├── search/        # usearch HNSW + BM25 + hybrid scoring
├── observer/      # file watcher
├── mcp-server/    # MCP JSON-RPC stdio
└── server/        # REST API (axum)
```

검색 점수:

```text
score = semantic(0.6) + keyword(0.15) + recency(0.15) + importance(0.10)
recency = exp(-ln(2)/30 * days_since_access)
```

## 검증 상태

현재 워크스페이스 기준:

- `cargo test --workspace`: **105 passed, 0 failed**
- `cargo build --workspace`: 성공
- `cargo clippy --workspace -- -D warnings -A clippy::arc-with-non-send-sync`: 성공

## CI/CD

워크플로우: `.github/workflows/ci.yml`

- 트리거: `push(main)`, `pull_request(main)`, `workflow_dispatch`, `schedule`(주 1회)
- `check` 잡: build/test/clippy/fmt + `cargo audit`
- `release-build` 잡: main 브랜치에서 release build 후 아티팩트 업로드

## 로컬 개발

```bash
export PATH="$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$HOME/.cargo/bin:$PATH"

cargo fmt --all
cargo clippy --workspace -- -D warnings -A clippy::arc-with-non-send-sync
cargo test --workspace
cargo build --release --workspace
```

모델 다운로드:

```bash
bash scripts/setup_model.sh
# 또는
python3 scripts/download_model.py
```

## 문서

- 통합/트리거 가이드: `SKILL.md`
- 개발 제약/아키텍처 참고: `CLAUDE.md`

## 라이선스

MIT
