#!/usr/bin/env bash
set -euo pipefail

REPO_URL="${REPO_URL:-https://github.com/NewTurn2017/oc-memory.git}"
INSTALL_DIR="${INSTALL_DIR:-$HOME/oc-memory}"
API_BASE="${API_BASE:-http://127.0.0.1:6342}"
CLAUDE_CONFIG="${CLAUDE_CONFIG:-$HOME/.claude/config.json}"
SERVICE_NAME="${SERVICE_NAME:-oc-memory.service}"
OS_NAME="$(uname -s | tr '[:upper:]' '[:lower:]')"

echo "[oc-memory] one-file setup started"
echo "[oc-memory] expected time: 10-25 minutes (network/model download speed dependent)"
echo "[oc-memory] major stages: deps -> build -> model -> service -> verification"

if [ "$OS_NAME" = "linux" ] && command -v apt-get >/dev/null 2>&1; then
  echo "[stage 1/5] installing system packages (linux)"
  apt-get update
  apt-get install -y build-essential pkg-config libssl-dev clang cmake curl python3 python3-venv git
elif [ "$OS_NAME" = "darwin" ] && command -v brew >/dev/null 2>&1; then
  echo "[stage 1/5] installing helper packages (macOS)"
  brew install cmake pkg-config openssl
fi

if ! command -v curl >/dev/null 2>&1; then
  echo "curl is required."
  exit 1
fi

if ! command -v git >/dev/null 2>&1; then
  echo "git is required."
  exit 1
fi

if ! command -v rustup >/dev/null 2>&1; then
  echo "[stage 1/5] installing rustup"
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
fi

source "$HOME/.cargo/env"

if [ -d "$INSTALL_DIR/.git" ]; then
  echo "[stage 2/5] updating existing repository: $INSTALL_DIR"
  git -C "$INSTALL_DIR" fetch origin
  git -C "$INSTALL_DIR" checkout main
  git -C "$INSTALL_DIR" pull --ff-only origin main
else
  echo "[stage 2/5] cloning repository: $REPO_URL -> $INSTALL_DIR"
  git clone "$REPO_URL" "$INSTALL_DIR"
fi

cd "$INSTALL_DIR"

echo "[stage 3/5] fetching dependencies and building release binaries"
cargo fetch
BUILDRS=$(find "$HOME/.cargo/registry/src" -path '*/lindera-ko-dic-*/build.rs' | head -1 || true)
if [ -n "$BUILDRS" ]; then
  if [ "$OS_NAME" = "darwin" ]; then
    sed -i '' 's|https://lindera.s3.ap-northeast-1.amazonaws.com/mecab-ko-dic-2.1.1-20180720.tar.gz|https://bitbucket.org/eunjeon/mecab-ko-dic/downloads/mecab-ko-dic-2.1.1-20180720.tar.gz|' "$BUILDRS" || true
  else
    sed -i 's|https://lindera.s3.ap-northeast-1.amazonaws.com/mecab-ko-dic-2.1.1-20180720.tar.gz|https://bitbucket.org/eunjeon/mecab-ko-dic/downloads/mecab-ko-dic-2.1.1-20180720.tar.gz|' "$BUILDRS" || true
  fi
fi

cargo build --release --workspace
echo "[stage 4/5] preparing model files (this can take several minutes)"
bash scripts/setup_model.sh

echo "[stage 5/5] configuring MCP + service"
python3 - "$CLAUDE_CONFIG" "$INSTALL_DIR" <<'PY'
import json
import sys
from pathlib import Path

cfg = Path(sys.argv[1])
root = Path(sys.argv[2])
cfg.parent.mkdir(parents=True, exist_ok=True)
data = {}
if cfg.exists():
    try:
        data = json.loads(cfg.read_text(encoding="utf-8"))
    except Exception:
        data = {}
data.setdefault("mcpServers", {})["memory"] = {
    "command": str(root / "target/release/oc-memory-mcp"),
    "args": []
}
cfg.write_text(json.dumps(data, ensure_ascii=False, indent=2), encoding="utf-8")
PY

ORT_LIB_DIR=""
if [ -d "$INSTALL_DIR/.venv-model/lib" ]; then
  ORT_LIB_DIR=$(find "$INSTALL_DIR/.venv-model/lib" -path '*/site-packages/onnxruntime/capi' | head -1 || true)
elif [ -d "$INSTALL_DIR/.venv-model/Lib/site-packages/onnxruntime/capi" ]; then
  ORT_LIB_DIR="$INSTALL_DIR/.venv-model/Lib/site-packages/onnxruntime/capi"
fi

if [ -d "$ORT_LIB_DIR" ]; then
  ORT_SO=$(ls "$ORT_LIB_DIR"/libonnxruntime.so.* 2>/dev/null | head -1 || true)
  if [ -n "$ORT_SO" ]; then
    ln -sf "$ORT_SO" "$ORT_LIB_DIR/libonnxruntime.so"
  fi
fi
[ -n "$ORT_LIB_DIR" ] || ORT_LIB_DIR="$INSTALL_DIR/.venv-model"

if command -v systemctl >/dev/null 2>&1; then
cat > "/etc/systemd/system/$SERVICE_NAME" <<EOF
[Unit]
Description=oc-memory REST Server
After=network.target

[Service]
Type=simple
User=root
WorkingDirectory=$INSTALL_DIR
ExecStart=$INSTALL_DIR/target/release/oc-memory-server
Restart=always
RestartSec=5
Environment=RUST_LOG=info
Environment=LD_LIBRARY_PATH=$ORT_LIB_DIR

[Install]
WantedBy=multi-user.target
EOF

systemctl daemon-reload
systemctl enable "$SERVICE_NAME"
systemctl restart "$SERVICE_NAME"
else
  pkill -f "oc-memory-server" >/dev/null 2>&1 || true
  nohup "$INSTALL_DIR/target/release/oc-memory-server" >/tmp/oc-memory-server.log 2>&1 &
fi

for _ in 1 2 3 4 5 6 7 8 9 10; do
  if curl -fsS "$API_BASE/health" >/dev/null 2>&1; then
    break
  fi
  sleep 1
done

curl -fsS "$API_BASE/api/v1/stats" >/tmp/oc-memory-stats.json

curl -fsS -X POST "$API_BASE/api/v1/memories" \
  -H 'content-type: application/json' \
  -d '{"title":"OpenClaw 자동회수 규칙","content":"세션 시작 시 oc-memory를 먼저 조회한다. prior-context 질문(지난번/이전/방금/뭐였지/기억/다시 알려줘)은 파일 탐색 전에 memory_search(index_only=true)를 우선 수행하고, 실패 시에만 grep으로 보조한다.","memory_type":"decision","priority":"high","tags":["openclaw","auto-recall","policy"]}' \
  >/tmp/oc-memory-policy-store.json

curl -fsS -X POST "$API_BASE/api/v1/search" \
  -H 'content-type: application/json' \
  -d '{"query":"OpenClaw 자동회수 규칙","limit":3,"index_only":true}' \
  >/tmp/oc-memory-policy-search.json

echo "[oc-memory] setup completed"
if command -v systemctl >/dev/null 2>&1; then
  echo "- Service: systemctl status $SERVICE_NAME --no-pager"
else
  echo "- Service log: /tmp/oc-memory-server.log"
fi
echo "- MCP config: $CLAUDE_CONFIG"
echo "- Stats: /tmp/oc-memory-stats.json"
echo "- Store check: /tmp/oc-memory-policy-store.json"
echo "- Search check: /tmp/oc-memory-policy-search.json"
echo ""
echo "[later verification commands]"
echo "curl -sS $API_BASE/api/v1/stats"
echo "bash $INSTALL_DIR/scripts/oc-memory-auto-recall.sh \"OpenClaw 자동회수 규칙\" 3"
echo "curl -sS -X POST $API_BASE/api/v1/search -H 'content-type: application/json' -d '{\"query\":\"지난번 자동회수 규칙\",\"limit\":3,\"index_only\":true}'"
