# oc-memory Skill

AI 비서의 장기 기억 시스템. 대화 컨텍스트, 사용자 선호도, 프로젝트 결정사항을 자동으로 저장하고 검색합니다.

## Trigger Conditions

다음 상황에서 자동으로 이 스킬을 활용하세요:

### 검색 트리거 (memory_search)
- **대화 시작**: 새 세션이 시작될 때 사용자 선호도와 최근 컨텍스트를 검색
- **"저번에", "이전에", "지난번"**: 과거 대화 참조 시 관련 기억 검색
- **"기억해?", "말했잖아"**: 사용자가 이전 대화를 참조할 때
- **프로젝트 관련 질문**: 해당 프로젝트의 결정사항, 아키텍처 정보 검색
- **"~에 대해 뭐 알아?"**: 특정 주제에 대한 저장된 지식 검색

### 저장 트리거 (memory_store)
- **사용자 선호도 표현**: "한국어로 해줘", "코드 리뷰는 영어로" → `preference` 타입
- **아키텍처/기술 결정**: "Rust로 하자", "BGE-m3-ko 쓰자" → `decision` 타입
- **중요한 사실 전달**: "서버는 Ubuntu 4CPU/8GB", "포트는 6342" → `fact` 타입
- **버그 해결**: 디버깅 과정과 해결책 → `bugfix` 타입
- **새로운 발견**: 예상치 못한 동작, 라이브러리 quirks → `discovery` 타입
- **"기억해 줘", "저장해"**: 명시적 저장 요청

## MCP Tools

### memory_search
```json
{
  "query": "검색할 내용 (자연어)",
  "limit": 5,
  "index_only": true
}
```
- **index_only=true** (기본): 제목/메타데이터만 반환 → 토큰 90% 절약
- 필요한 항목만 `memory_get`으로 전체 내용 조회

### memory_store
```json
{
  "content": "저장할 내용",
  "title": "제목 (간결하게)",
  "memory_type": "observation|decision|preference|fact|task|session|bugfix|discovery",
  "priority": "low|medium|high",
  "tags": ["tag1", "tag2"]
}
```

### memory_get
```json
{
  "id": "메모리 ID"
}
```
- index_only 검색 후 필요한 항목의 전체 내용 조회

### memory_delete
```json
{
  "id": "삭제할 메모리 ID"
}
```

### memory_stats
```json
{}
```
- 총 메모리 수, 인덱스 상태, 검색 모드 확인

## Progressive Disclosure Pattern

토큰 절약을 위해 항상 다음 패턴을 따르세요:

```
1. memory_search(query, index_only=true, limit=5)
   → 제목 + 메타데이터만 반환 (~50 tokens/item)

2. 관련 있는 항목만 memory_get(id)으로 전체 조회
   → 필요한 컨텐츠만 로드 (~200-500 tokens/item)

3. 불필요한 항목은 조회하지 않음
   → 90%+ 토큰 절약
```

## Memory Type 가이드

| 타입 | 언제 사용 | 우선순위 기본값 |
|------|----------|---------------|
| `preference` | 사용자 작업 스타일, 언어, 도구 선호 | high |
| `decision` | 아키텍처, 기술 스택, 설계 결정 | high |
| `fact` | 환경 정보, 서버 스펙, 계정 정보 | medium |
| `observation` | 일반적인 관찰, 메모 | medium |
| `bugfix` | 버그 원인과 해결책 | medium |
| `discovery` | 라이브러리 quirks, 예상 밖 동작 | medium |
| `task` | 진행 중인 작업, TODO | low |
| `session` | 세션 요약 | low |

## 자동 행동 예시

### 세션 시작 시
```
→ memory_search("사용자 선호도", limit=3, index_only=true)
→ memory_search("최근 프로젝트 컨텍스트", limit=5, index_only=true)
→ 필요한 항목만 memory_get()
```

### 사용자가 결정을 내릴 때
```
사용자: "Rust로 통합하고 싶어"
→ memory_store(
    content="사용자가 프로젝트를 Rust로 통합하기로 결정. 이유: 성능, 안전성, 로컬 실행 요구사항.",
    title="Rust 통합 결정",
    memory_type="decision",
    priority="high",
    tags=["rust", "architecture"]
  )
```

### 과거 참조 시
```
사용자: "저번에 어떤 모델 쓰기로 했지?"
→ memory_search("모델 결정", limit=5, index_only=true)
→ 관련 항목 memory_get()
→ "BGE-m3-ko INT8 양자화 모델을 사용하기로 결정하셨습니다."
```

## Setup

```bash
# 1. 모델 다운로드
bash scripts/setup_model.sh

# 2. 빌드
cargo build --release -p oc-mcp-server

# 3. OpenClaw/Claude Code 설정
# ~/.claude/config.json 또는 해당 설정 파일에 추가:
{
  "mcpServers": {
    "memory": {
      "command": "/path/to/oc-memory-mcp",
      "args": []
    }
  }
}
```

## Constraints

- **토큰 절약 최우선**: 항상 `index_only=true`로 먼저 검색
- **중복 저장 금지**: 같은 정보를 반복 저장하지 않음 (먼저 검색)
- **자연스럽게**: 메모리 사용을 사용자에게 과도하게 알리지 않음
- **선택적 저장**: 모든 대화를 저장하지 않음 — 가치 있는 정보만
- **한국어 우선**: 한국어 컨텐츠가 주이므로 한국어로 저장
