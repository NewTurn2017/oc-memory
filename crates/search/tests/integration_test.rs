//! Integration tests for the full store → index → search pipeline.
//! These tests run WITHOUT the embedding model (keyword-only mode).

use oc_core::Storage;
use oc_core::models::{Memory, MemoryMetadata, MemoryType, Priority, SearchQuery};
use oc_search::bm25::Bm25Index;
use oc_search::hybrid::HybridSearch;
use oc_search::scoring::Scorer;
use oc_search::vector::VectorIndex;
use std::sync::Arc;

/// Helper: create in-memory storage + hybrid search engine
fn create_test_engine() -> (Arc<Storage>, HybridSearch) {
    let storage = Arc::new(Storage::in_memory().unwrap());
    let vector_index = VectorIndex::new(4); // 4-dim for testing
    let bm25_index = Bm25Index::in_memory().unwrap();
    let scorer = Scorer::default();
    let search = HybridSearch::new(storage.clone(), vector_index, bm25_index, scorer);
    (storage, search)
}

/// Helper: create a memory with optional fake embedding
fn make_memory(title: &str, content: &str, tags: &[&str], embedding: Option<Vec<f32>>) -> Memory {
    let mut memory = Memory::new(
        content.to_string(),
        title.to_string(),
        MemoryMetadata {
            memory_type: MemoryType::Observation,
            priority: Priority::Medium,
            tags: tags.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        },
    );
    memory.embedding = embedding;
    memory
}

#[test]
fn test_store_and_keyword_search() {
    let (storage, mut search) = create_test_engine();

    // Store 3 memories
    let m1 = make_memory(
        "Rust 프로젝트 설정",
        "Cargo workspace를 사용하여 Rust 프로젝트를 구성합니다",
        &["rust", "cargo"],
        None,
    );
    let m2 = make_memory(
        "Python 환경 설정",
        "virtualenv를 사용하여 Python 개발 환경을 설정합니다",
        &["python", "virtualenv"],
        None,
    );
    let m3 = make_memory(
        "Docker 배포",
        "Docker compose를 사용하여 서비스를 배포합니다",
        &["docker", "deploy"],
        None,
    );

    // Insert into storage
    storage.insert(&m1).unwrap();
    storage.insert(&m2).unwrap();
    storage.insert(&m3).unwrap();

    // Index in search
    search.index_memory(&m1).unwrap();
    search.index_memory(&m2).unwrap();
    search.index_memory(&m3).unwrap();

    assert_eq!(storage.count().unwrap(), 3);
    assert_eq!(search.indexed_count(), 0); // No embeddings → vector index empty

    // Search with keyword only (zero embedding)
    let query = SearchQuery {
        query: "Rust Cargo".to_string(),
        limit: 5,
        ..Default::default()
    };
    let zero_emb = vec![0f32; 4];
    let results = search.search(&zero_emb, &query).unwrap();

    // BM25 should find the Rust memory
    assert!(!results.is_empty(), "Keyword search should return results");
    assert_eq!(results[0].memory.title, "Rust 프로젝트 설정");
}

#[test]
fn test_store_and_vector_search() {
    let (storage, mut search) = create_test_engine();

    // Create memories with fake embeddings (4-dim)
    let m1 = make_memory(
        "벡터 검색",
        "벡터 유사도 기반 검색 구현",
        &["vector"],
        Some(vec![1.0, 0.0, 0.0, 0.0]),
    );
    let m2 = make_memory(
        "키워드 검색",
        "BM25 키워드 검색 구현",
        &["keyword"],
        Some(vec![0.0, 1.0, 0.0, 0.0]),
    );
    let m3 = make_memory(
        "하이브리드 검색",
        "벡터와 키워드를 합친 하이브리드 검색",
        &["hybrid"],
        Some(vec![0.7, 0.7, 0.0, 0.0]),
    );

    storage.insert(&m1).unwrap();
    storage.insert(&m2).unwrap();
    storage.insert(&m3).unwrap();

    search.index_memory(&m1).unwrap();
    search.index_memory(&m2).unwrap();
    search.index_memory(&m3).unwrap();

    assert_eq!(search.indexed_count(), 3);

    // Query close to m1's embedding
    let query = SearchQuery {
        query: "벡터".to_string(),
        limit: 3,
        ..Default::default()
    };
    let query_emb = vec![0.9, 0.1, 0.0, 0.0]; // Close to m1
    let results = search.search(&query_emb, &query).unwrap();

    assert!(!results.is_empty());
    // Vector similarity should rank m1 highest (closest to query)
    assert_eq!(results[0].memory.title, "벡터 검색");
}

#[test]
fn test_index_only_strips_content() {
    let (storage, mut search) = create_test_engine();

    let m1 = make_memory(
        "비밀 메모리",
        "이것은 매우 긴 컨텐츠입니다. 인덱스 모드에서는 반환되지 않아야 합니다.",
        &["secret"],
        Some(vec![1.0, 0.0, 0.0, 0.0]),
    );

    storage.insert(&m1).unwrap();
    search.index_memory(&m1).unwrap();

    let query = SearchQuery {
        query: "비밀".to_string(),
        limit: 5,
        index_only: true,
        ..Default::default()
    };
    let results = search.search(&[1.0, 0.0, 0.0, 0.0], &query).unwrap();

    assert!(!results.is_empty());
    // Content should be stripped in index_only mode
    assert!(
        results[0].memory.content.is_empty(),
        "index_only should strip content"
    );
    // But title and metadata should remain
    assert_eq!(results[0].memory.title, "비밀 메모리");
}

#[test]
fn test_delete_removes_from_both_indices() {
    let (storage, mut search) = create_test_engine();

    let m1 = make_memory(
        "삭제 대상",
        "이 메모리는 삭제됩니다",
        &["delete"],
        Some(vec![1.0, 0.0, 0.0, 0.0]),
    );
    let id = m1.id.clone();

    storage.insert(&m1).unwrap();
    search.index_memory(&m1).unwrap();

    assert_eq!(storage.count().unwrap(), 1);
    assert_eq!(search.indexed_count(), 1);

    // Delete
    search.remove_memory(&id).unwrap();
    storage.delete(&id).unwrap();

    assert_eq!(storage.count().unwrap(), 0);
    assert_eq!(search.indexed_count(), 0);
}

#[test]
fn test_score_breakdown_present() {
    let (storage, mut search) = create_test_engine();

    let m1 = make_memory(
        "점수 테스트",
        "이 메모리의 점수 분해를 확인합니다",
        &["score"],
        Some(vec![1.0, 0.0, 0.0, 0.0]),
    );

    storage.insert(&m1).unwrap();
    search.index_memory(&m1).unwrap();

    let query = SearchQuery {
        query: "점수".to_string(),
        limit: 5,
        ..Default::default()
    };
    let results = search.search(&[0.9, 0.1, 0.0, 0.0], &query).unwrap();

    assert!(!results.is_empty());
    let breakdown = &results[0].score_breakdown;

    // Score breakdown should have semantic + keyword + recency + importance
    assert!(results[0].score > 0.0, "Score should be positive");
    assert!(
        breakdown.semantic >= 0.0,
        "Semantic score should be non-negative"
    );
    assert!(
        breakdown.recency >= 0.0,
        "Recency score should be non-negative"
    );
    assert!(
        breakdown.importance >= 0.0,
        "Importance score should be non-negative"
    );
}

#[test]
fn test_korean_keyword_search() {
    let (storage, mut search) = create_test_engine();

    let m1 = make_memory(
        "사용자 선호도",
        "사용자는 한국어 대화를 선호합니다. 코드 리뷰는 영어로 합니다.",
        &["preference", "korean"],
        None,
    );
    let m2 = make_memory(
        "프로젝트 결정사항",
        "BGE-m3-ko 모델을 INT8 양자화하여 사용하기로 결정했습니다",
        &["decision", "model"],
        None,
    );

    storage.insert(&m1).unwrap();
    storage.insert(&m2).unwrap();
    search.index_memory(&m1).unwrap();
    search.index_memory(&m2).unwrap();

    // Search using exact tokens present in content.
    // Note: tantivy default tokenizer splits on whitespace/punctuation,
    // so Korean agglutinative forms (한국어로) won't match (한국어).
    // We use a token that exists exactly in the content.
    let query = SearchQuery {
        query: "한국어".to_string(),
        limit: 5,
        ..Default::default()
    };
    let zero_emb = vec![0f32; 4];
    let results = search.search(&zero_emb, &query).unwrap();

    assert!(!results.is_empty(), "Korean keyword search should work");
    assert_eq!(results[0].memory.title, "사용자 선호도");

    // Search for BGE model decision
    let query2 = SearchQuery {
        query: "BGE INT8".to_string(),
        limit: 5,
        ..Default::default()
    };
    let results2 = search.search(&zero_emb, &query2).unwrap();
    assert!(!results2.is_empty(), "BGE keyword search should work");
    assert_eq!(results2[0].memory.title, "프로젝트 결정사항");
}
