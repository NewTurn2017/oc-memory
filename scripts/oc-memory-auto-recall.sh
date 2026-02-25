#!/usr/bin/env bash
set -euo pipefail

QUERY="${1:-최근 작업}"
LIMIT="${2:-3}"
API_BASE="${OC_MEMORY_API_BASE:-http://127.0.0.1:6342}"

PAYLOAD=$(python3 - "$QUERY" "$LIMIT" <<'PY'
import json
import sys

query = sys.argv[1]
limit = int(sys.argv[2])
print(json.dumps({"query": query, "limit": limit, "index_only": True}, ensure_ascii=False))
PY
)

RESP=$(curl -sS -X POST "$API_BASE/api/v1/search" \
  -H 'content-type: application/json' \
  -d "$PAYLOAD")

python3 - "$RESP" <<'PY'
import json
import sys

raw = sys.argv[1]
data = json.loads(raw)
if not data.get("success"):
    print(f"search failed: {data.get('error', 'unknown error')}")
    sys.exit(1)

rows = data.get("data") or []
if not rows:
    print("no results")
    sys.exit(0)

for i, row in enumerate(rows, 1):
    m = row.get("memory", {})
    title = m.get("title") or "(no title)"
    content = (m.get("content") or "").replace("\n", " ")
    content = content[:140] + ("..." if len(content) > 140 else "")
    score = row.get("score", 0)
    print(f"[{i}] {title} :: {content} (score={score:.4f})")
PY
