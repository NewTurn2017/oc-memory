# oc-memory Skill

`SKILL.md` 하나만 보고 OpenClaw/Claude Code에서 oc-memory를 설치, 실행, 검증(벡터 검색 포함)할 수 있도록 작성된 원스톱 가이드입니다.

---

## 0) 목표 상태

- GitHub에서 코드 클론 완료
- `oc-memory-mcp`, `oc-memory-server` 빌드 완료
- 모델/토크나이저 다운로드 완료
- OpenClaw(또는 Claude Code) MCP 서버 등록 완료
- REST 서버에서 임베딩/하이브리드 검색 활성 확인
  - `has_embedder: true`
  - `search_mode: "hybrid"`
  - 저장 응답 `has_embedding: true`
- 검색 응답 `score_breakdown.semantic > 0`

---

## 빠른 적용 (원파일)

Telegram 등으로 단일 파일 전달 후 바로 적용할 경우 아래 파일 하나만 실행하세요.

```bash
bash openclaw-onefile-setup.sh
```

예상 시간: **10~25분** (네트워크/모델 다운로드 속도에 따라 변동)

설치 후 나중에 확인:

```bash
curl -sS http://127.0.0.1:6342/api/v1/stats
bash scripts/oc-memory-auto-recall.sh "OpenClaw 자동회수 규칙" 3
```

환경별 경로를 바꿔야 하면 환경변수로 지정 가능합니다.

```bash
INSTALL_DIR=/root/oc-memory API_BASE=http://127.0.0.1:6342 bash openclaw-onefile-setup.sh
```

플랫폼 동작:

- Linux(Ubuntu): apt + systemd 자동 설정
- macOS: brew 패키지 보조 설치 후 서버를 `nohup`으로 실행
- Windows(Git Bash/WSL): git/curl/rust/python이 이미 설치되어 있으면 나머지 단계를 자동 진행하고 서버는 `nohup` 경로로 실행

---

## 1) 클라우드 선행 설치 (Ubuntu 24.04)

```bash
sudo apt update
sudo apt install -y build-essential pkg-config libssl-dev clang cmake curl python3 python3-venv

curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"

rustc --version
cargo --version
python3 --version
```

---

## 2) GitHub 클론

```bash
git clone https://github.com/NewTurn2017/oc-memory.git
cd oc-memory
```

---

## 3) lindera-ko-dic URL 임시 패치 (필요 시)

`cargo clean`/fresh clone에서 `NoSuchBucket`, `invalid gzip header`가 나면 먼저 적용합니다.

```bash
cargo fetch
BUILDRS=$(find ~/.cargo/registry/src -path '*/lindera-ko-dic-*/build.rs' | head -1)

# Linux
sed -i 's|https://lindera.s3.ap-northeast-1.amazonaws.com/mecab-ko-dic-2.1.1-20180720.tar.gz|https://bitbucket.org/eunjeon/mecab-ko-dic/downloads/mecab-ko-dic-2.1.1-20180720.tar.gz|' "$BUILDRS"

# macOS
# sed -i '' 's|https://lindera.s3.ap-northeast-1.amazonaws.com/mecab-ko-dic-2.1.1-20180720.tar.gz|https://bitbucket.org/eunjeon/mecab-ko-dic/downloads/mecab-ko-dic-2.1.1-20180720.tar.gz|' "$BUILDRS"
```

중요: 이 방법은 Cargo registry 캐시 수정이라 임시 우회입니다. 장기적으로는 레포 레벨 패치(`patch.crates-io`) 또는 의존성 업그레이드가 필요합니다.

---

## 4) 빌드/테스트

```bash
source "$HOME/.cargo/env"

cargo test --workspace
cargo build --release --workspace

ls -lh target/release/oc-memory-mcp target/release/oc-memory-server
```

---

## 5) 모델/토크나이저 다운로드 (필수)

권장 경로(venv + prebuilt ONNX):

```bash
bash scripts/setup_model.sh
```

대안:

```bash
# prebuilt ONNX 직접 다운로드
python3 scripts/download_model.py

# 로컬 변환/양자화 경로
python3 scripts/download_model.py --convert
```

확인:

```bash
ls -lh ~/.local/share/oc-memory/models
# bge-m3-ko-int8.onnx
# tokenizer.json
```

---

## 6) MCP 등록 (OpenClaw/Claude Code)

`~/.claude/config.json`에 `mcpServers.memory` 등록:

```bash
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
print(f"Updated: {cfg}")
PY
```

---

## 7) REST 서버 실행 (벡터 검증용)

```bash
./target/release/oc-memory-server
```

다른 터미널에서 검증:

```bash
curl -sS http://127.0.0.1:6342/health
curl -sS http://127.0.0.1:6342/api/v1/stats
```

기대값: `has_embedder: true`, `search_mode: "hybrid"`

---

## 8) 벡터 검색 실제 검증

### 8-1. 메모리 저장

```bash
curl -sS -X POST http://127.0.0.1:6342/api/v1/memories \
  -H 'content-type: application/json' \
  -d '{"title":"벡터테스트","content":"광안리 해변 산책 일정","memory_type":"observation","priority":"high","tags":["travel","korea"]}'
```

기대값: `has_embedding: true`

### 8-2. 검색

```bash
curl -sS -X POST http://127.0.0.1:6342/api/v1/search \
  -H 'content-type: application/json' \
  -d '{"query":"해변 산책 일정","limit":3,"index_only":true}'
```

기대값:

- `data[].score_breakdown.semantic` 값이 0보다 큼
- 결과가 하이브리드 점수 기준으로 정렬됨

### 8-3. 통계 재확인

```bash
curl -sS http://127.0.0.1:6342/api/v1/stats
```

기대값: `total_memories`/`indexed_count` 증가

---

## 9) 자동 회수(E2E) 설정

새 세션에서 "지난번/이전/방금/뭐였지" 류 질문이 들어올 때 먼저 oc-memory를 조회하도록 운영할 수 있습니다.

### 9-1. 자동 회수 스크립트

```bash
bash scripts/oc-memory-auto-recall.sh "검증키" 3
```

출력 형식:

```text
[1] 제목 :: 요약문 (score=...)
[2] 제목 :: 요약문 (score=...)
```

### 9-2. 검증 데이터 저장

```bash
curl -sS -X POST http://127.0.0.1:6342/api/v1/memories \
  -H 'content-type: application/json' \
  -d '{"title":"자동회수 설정 완료","content":"자동 회수 방식: 새 세션에서 prior work 질문 시 oc-memory를 먼저 조회한 뒤 답변한다. 검증키 auto-recall-4412","memory_type":"decision","priority":"high"}'
```

확인:

```bash
bash scripts/oc-memory-auto-recall.sh "auto-recall-4412" 3
```

### 9-3. OpenClaw 전용 메모리(Always-on)로 규칙 고정

`AGENTS.md` 대신 OpenClaw 전용 메모리에 자동 회수 규칙을 저장해 세션 시작마다 우선 참조하도록 운영합니다.

저장 예시(REST):

```bash
curl -sS -X POST http://127.0.0.1:6342/api/v1/memories \
  -H 'content-type: application/json' \
  -d '{"title":"OpenClaw 자동회수 규칙","content":"세션 시작 시 oc-memory를 먼저 조회한다. prior-context 질문(지난번/이전/방금/뭐였지/기억/다시 알려줘)은 파일 탐색 전에 memory_search(index_only=true)를 우선 수행하고, 실패 시에만 grep으로 보조한다.","memory_type":"decision","priority":"high","tags":["openclaw","auto-recall","policy"]}'
```

세션 시작 조회 예시:

```bash
bash scripts/oc-memory-auto-recall.sh "OpenClaw 자동회수 규칙" 3
```

---

## 10) systemd 서비스 전환 예시 (운영)

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
sudo systemctl status oc-memory.service --no-pager
```

---

## 11) MCP 도구 요약

- `memory_search`: 하이브리드 검색
- `memory_store`: 메모리 저장
- `memory_get`: ID 기반 조회
- `memory_delete`: 삭제
- `memory_stats`: 상태 확인

---

## 12) 트리거 가이드 (에이전트 행동)

### 검색 트리거

- 대화 시작 시 컨텍스트 복원
- "저번에/이전에/지난번" 참조
- 프로젝트 결정사항/구조 질문

### 저장 트리거

- 선호도: `preference`
- 기술/아키텍처 결정: `decision`
- 환경 사실: `fact`
- 버그 해결: `bugfix`

### 토큰 절약 패턴

1. `memory_search(index_only=true)`
2. 필요한 항목만 `memory_get`
3. 불필요한 전체 조회 생략

---

## 13) 자주 발생하는 장애

1. `rustc: command not found`
   - `rustup` 설치 후 `source "$HOME/.cargo/env"`

2. `linker cc not found`
   - `build-essential pkg-config libssl-dev clang cmake` 설치

3. `lindera-ko-dic` URL 에러
   - 3번 단계 임시 패치 적용

4. `externally-managed-environment` (PEP668)
   - 시스템 `pip3` 대신 `.venv-model` 사용 (`setup_model.sh`)

5. `libonnxruntime.so` 로딩 실패
   - systemd `LD_LIBRARY_PATH` 확인 및 재시작
