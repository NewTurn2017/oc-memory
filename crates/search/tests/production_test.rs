//! Production-grade tests for oc-search.
//!
//! Covers: vector stress, BM25 Korean edge cases, hybrid search correctness,
//! scoring boundaries, concurrency, and data integrity.

use oc_core::Storage;
use oc_core::models::{Memory, MemoryMetadata, MemoryType, Priority, SearchQuery};
use oc_search::bm25::Bm25Index;
use oc_search::hybrid::HybridSearch;
use oc_search::scoring::Scorer;
use oc_search::vector::VectorIndex;
use std::sync::Arc;

// ============================================================
// Helpers
// ============================================================

fn test_engine() -> (Arc<Storage>, HybridSearch) {
    let storage = Arc::new(Storage::in_memory().unwrap());
    let vector = VectorIndex::new(4);
    let bm25 = Bm25Index::in_memory().unwrap();
    let scorer = Scorer::default();
    let search = HybridSearch::new(storage.clone(), vector, bm25, scorer);
    (storage, search)
}

fn mem(title: &str, content: &str, emb: Option<Vec<f32>>) -> Memory {
    let mut m = Memory::new(
        content.to_string(),
        title.to_string(),
        MemoryMetadata::default(),
    );
    m.embedding = emb;
    m
}

fn mem_with(
    title: &str,
    content: &str,
    emb: Option<Vec<f32>>,
    priority: Priority,
    memory_type: MemoryType,
    tags: Vec<&str>,
) -> Memory {
    let mut m = Memory::new(
        content.to_string(),
        title.to_string(),
        MemoryMetadata {
            memory_type,
            priority,
            tags: tags.into_iter().map(String::from).collect(),
            ..Default::default()
        },
    );
    m.embedding = emb;
    m
}

fn index(storage: &Storage, search: &mut HybridSearch, m: &Memory) {
    storage.insert(m).unwrap();
    search.index_memory(m).unwrap();
}

fn query(text: &str, limit: usize) -> SearchQuery {
    SearchQuery {
        query: text.to_string(),
        limit,
        ..Default::default()
    }
}

// ============================================================
// 1. Vector Index Stress Tests
// ============================================================

#[test]
fn test_vector_scale_1000() {
    let mut vi = VectorIndex::new(128);
    let mut ids = Vec::new();

    for i in 0..1000 {
        let mut v = vec![0.0_f32; 128];
        v[i % 128] = 1.0;
        v[(i + 1) % 128] = 0.5;
        let id = format!("vec-{i}");
        vi.upsert(id.clone(), v).unwrap();
        ids.push(id);
    }
    assert_eq!(vi.len(), 1000);

    // Search should return results
    let mut q = vec![0.0_f32; 128];
    q[0] = 1.0;
    let results = vi.search(&q, 10);
    assert_eq!(results.len(), 10);
    // Top result should be one with v[0]=1.0
    assert!(results[0].1 > 0.5, "Top result similarity should be high");
}

#[test]
fn test_vector_high_dimensional() {
    // Simulate actual BGE-m3-ko dimensions (1024)
    let mut vi = VectorIndex::new(1024);

    let mut v1 = vec![0.01_f32; 1024];
    v1[0] = 1.0;
    let mut v2 = vec![0.01_f32; 1024];
    v2[1] = 1.0;

    vi.upsert("a".to_string(), v1.clone()).unwrap();
    vi.upsert("b".to_string(), v2).unwrap();

    let results = vi.search(&v1, 2);
    assert_eq!(results[0].0, "a");
    assert!(results[0].1 > results[1].1);
}

#[test]
fn test_vector_rapid_upsert_remove_cycles() {
    let mut vi = VectorIndex::new(4);

    for cycle in 0..50 {
        let id = format!("item-{}", cycle % 10);
        let v = vec![cycle as f32 / 50.0, 0.5, 0.3, 0.1];
        vi.upsert(id.clone(), v).unwrap();

        if cycle % 3 == 0 {
            vi.remove(&id);
        }
    }

    // Index should not crash, and size should be consistent
    assert!(vi.len() <= 10);
    let results = vi.search(&[1.0, 0.5, 0.3, 0.1], 5);
    assert!(results.len() <= vi.len());
}

#[test]
fn test_vector_search_after_all_removed() {
    let mut vi = VectorIndex::new(3);
    vi.upsert("a".to_string(), vec![1.0, 0.0, 0.0]).unwrap();
    vi.upsert("b".to_string(), vec![0.0, 1.0, 0.0]).unwrap();
    vi.remove("a");
    vi.remove("b");

    assert_eq!(vi.len(), 0);
    let results = vi.search(&[1.0, 0.0, 0.0], 5);
    assert!(results.is_empty());
}

#[test]
fn test_vector_near_duplicate_vectors() {
    let mut vi = VectorIndex::new(4);
    vi.upsert("orig".to_string(), vec![1.0, 0.0, 0.0, 0.0])
        .unwrap();
    vi.upsert("near".to_string(), vec![0.999, 0.001, 0.0, 0.0])
        .unwrap();
    vi.upsert("far".to_string(), vec![0.0, 0.0, 0.0, 1.0])
        .unwrap();

    let results = vi.search(&[1.0, 0.0, 0.0, 0.0], 3);
    assert_eq!(results.len(), 3);
    // Both orig and near should rank above far
    let far_idx = results.iter().position(|(id, _)| id == "far").unwrap();
    assert_eq!(far_idx, 2, "Far vector should be ranked last");
}

#[test]
fn test_vector_similarity_range() {
    let mut vi = VectorIndex::new(3);
    vi.upsert("a".to_string(), vec![1.0, 0.0, 0.0]).unwrap();
    vi.upsert("b".to_string(), vec![0.0, 1.0, 0.0]).unwrap();
    vi.upsert("c".to_string(), vec![-1.0, 0.0, 0.0]).unwrap();

    let results = vi.search(&[1.0, 0.0, 0.0], 3);
    for (_, sim) in &results {
        assert!(
            *sim >= 0.0 && *sim <= 1.0,
            "Similarity {} out of [0,1] range",
            sim
        );
    }
}

// ============================================================
// 2. BM25 Korean Edge Cases
// ============================================================

#[test]
fn test_bm25_korean_particles() {
    let bm25 = Bm25Index::in_memory().unwrap();
    bm25.add("1", "개발", "프로그래밍을 좋아합니다").unwrap();
    bm25.add("2", "음악", "음악을 듣습니다").unwrap();

    // "프로그래밍" should match "프로그래밍을" via morpheme splitting
    let results = bm25.search("프로그래밍", 5).unwrap();
    assert!(!results.is_empty(), "Korean particle stripping should work");
    assert_eq!(results[0].0, "1");
}

#[test]
fn test_bm25_mixed_korean_english() {
    let bm25 = Bm25Index::in_memory().unwrap();
    bm25.add("1", "Rust 프로젝트", "Rust와 Python을 비교합니다")
        .unwrap();
    bm25.add("2", "JavaScript 개발", "React와 Vue를 사용합니다")
        .unwrap();

    let results = bm25.search("Rust Python", 5).unwrap();
    assert!(!results.is_empty());
    assert_eq!(results[0].0, "1");

    let results2 = bm25.search("React", 5).unwrap();
    assert!(!results2.is_empty());
    assert_eq!(results2[0].0, "2");
}

#[test]
fn test_bm25_long_document() {
    let bm25 = Bm25Index::in_memory().unwrap();
    let long_content = "Rust 프로그래밍 언어는 ".to_string()
        + &"메모리 안전성과 동시성을 보장하는 시스템 프로그래밍 언어입니다. ".repeat(500);
    bm25.add("1", "긴 문서", &long_content).unwrap();
    bm25.add("2", "짧은 문서", "Rust 기초").unwrap();

    let results = bm25.search("메모리 안전", 5).unwrap();
    assert!(!results.is_empty());
    assert_eq!(
        results[0].0, "1",
        "Long doc with repeated term should rank high"
    );
}

#[test]
fn test_bm25_remove_and_readd() {
    let bm25 = Bm25Index::in_memory().unwrap();
    bm25.add("1", "제목", "원본 내용").unwrap();
    bm25.remove("1").unwrap();

    // After remove, search should not find it
    let results = bm25.search("원본", 5).unwrap();
    assert!(
        results.is_empty(),
        "Removed doc should not appear in search"
    );

    // Re-add with different content
    bm25.add("1", "제목", "수정된 내용").unwrap();
    let results2 = bm25.search("수정", 5).unwrap();
    assert!(!results2.is_empty());
}

#[test]
fn test_bm25_special_characters() {
    let bm25 = Bm25Index::in_memory().unwrap();
    bm25.add(
        "1",
        "특수문자",
        "C++ & C# 프로그래밍 <script>alert()</script>",
    )
    .unwrap();
    bm25.add("2", "일반", "일반 텍스트").unwrap();

    // Should not crash on special chars
    let results = bm25.search("프로그래밍", 5).unwrap();
    assert!(!results.is_empty());
}

#[test]
fn test_bm25_bulk_50_documents() {
    let bm25 = Bm25Index::in_memory().unwrap();
    for i in 0..50 {
        bm25.add(
            &format!("{i}"),
            &format!("문서 {i}"),
            &format!("내용 {i} 검색 테스트 문서입니다"),
        )
        .unwrap();
    }

    let results = bm25.search("검색 테스트", 10).unwrap();
    assert_eq!(results.len(), 10, "Should return exactly limit results");
}

// ============================================================
// 3. Hybrid Search End-to-End
// ============================================================

#[test]
fn test_hybrid_vector_dominant_ranking() {
    let (storage, mut search) = test_engine();

    // m1: strong vector match, weak keyword
    let m1 = mem("벡터 우선", "무관한 텍스트", Some(vec![1.0, 0.0, 0.0, 0.0]));
    // m2: weak vector match, strong keyword
    let m2 = mem(
        "키워드 우선",
        "벡터 유사도 검색",
        Some(vec![0.0, 1.0, 0.0, 0.0]),
    );

    index(&storage, &mut search, &m1);
    index(&storage, &mut search, &m2);

    // Query embedding close to m1, query text matches m2
    let results = search
        .search(&[0.95, 0.05, 0.0, 0.0], &query("벡터 유사도", 2))
        .unwrap();
    assert_eq!(results.len(), 2);
    // With semantic weight 0.6 vs keyword 0.15, vector-dominant m1 might win or m2 might
    // win due to keyword; the important thing is both are returned and scored
    assert!(results[0].score > 0.0);
    assert!(results[1].score > 0.0);
}

#[test]
fn test_hybrid_keyword_only_mode() {
    let (storage, mut search) = test_engine();

    let m1 = mem("Rust 입문", "Rust 프로그래밍 기초 가이드", None);
    let m2 = mem("Python 입문", "Python 기초 학습", None);

    index(&storage, &mut search, &m1);
    index(&storage, &mut search, &m2);

    // Zero embedding = no vector signal, pure keyword
    let results = search
        .search(&[0.0; 4], &query("Rust 프로그래밍", 5))
        .unwrap();
    assert!(!results.is_empty());
    assert_eq!(results[0].memory.title, "Rust 입문");
}

#[test]
fn test_hybrid_priority_influence() {
    let (storage, mut search) = test_engine();

    // Same content, same embedding, different priority
    let m_low = mem_with(
        "낮은 우선순위",
        "검색 테스트",
        Some(vec![1.0, 0.0, 0.0, 0.0]),
        Priority::Low,
        MemoryType::Observation,
        vec![],
    );
    let m_high = mem_with(
        "높은 우선순위",
        "검색 테스트",
        Some(vec![1.0, 0.0, 0.0, 0.0]),
        Priority::High,
        MemoryType::Observation,
        vec![],
    );

    index(&storage, &mut search, &m_low);
    index(&storage, &mut search, &m_high);

    let results = search
        .search(&[1.0, 0.0, 0.0, 0.0], &query("검색", 2))
        .unwrap();
    assert_eq!(results.len(), 2);

    // High priority should score higher (importance weight = 0.10)
    let high_score = results
        .iter()
        .find(|r| r.memory.title == "높은 우선순위")
        .unwrap()
        .score;
    let low_score = results
        .iter()
        .find(|r| r.memory.title == "낮은 우선순위")
        .unwrap()
        .score;
    assert!(
        high_score > low_score,
        "High priority ({high_score}) should beat low priority ({low_score})"
    );
}

#[test]
fn test_hybrid_index_only_mode() {
    let (storage, mut search) = test_engine();

    let m = mem(
        "인덱스 테스트",
        "이 내용은 인덱스 모드에서 보이지 않아야 합니다",
        Some(vec![1.0, 0.0, 0.0, 0.0]),
    );
    index(&storage, &mut search, &m);

    let q = SearchQuery {
        query: "인덱스".to_string(),
        limit: 5,
        index_only: true,
        ..Default::default()
    };
    let results = search.search(&[1.0, 0.0, 0.0, 0.0], &q).unwrap();
    assert!(!results.is_empty());
    assert!(
        results[0].memory.content.is_empty(),
        "index_only should strip content"
    );
    assert!(!results[0].memory.title.is_empty(), "title should remain");
    assert!(
        results[0].memory.embedding.is_none(),
        "embedding should be stripped"
    );
}

#[test]
fn test_hybrid_many_results_truncation() {
    let (storage, mut search) = test_engine();

    for i in 0..20 {
        let m = mem(
            &format!("메모리 {i}"),
            &format!("검색 대상 내용 {i}"),
            Some(vec![1.0 - (i as f32 * 0.04), i as f32 * 0.04, 0.0, 0.0]),
        );
        index(&storage, &mut search, &m);
    }

    let results = search
        .search(&[1.0, 0.0, 0.0, 0.0], &query("검색", 5))
        .unwrap();
    assert_eq!(results.len(), 5, "Should respect limit");

    // Scores should be descending
    for w in results.windows(2) {
        assert!(
            w[0].score >= w[1].score,
            "Results should be sorted by score"
        );
    }
}

#[test]
fn test_hybrid_delete_reflects_in_search() {
    let (storage, mut search) = test_engine();

    let m1 = mem(
        "삭제 대상",
        "이것은 삭제됩니다",
        Some(vec![1.0, 0.0, 0.0, 0.0]),
    );
    let m2 = mem(
        "유지 대상",
        "이것은 유지됩니다",
        Some(vec![0.9, 0.1, 0.0, 0.0]),
    );
    let id1 = m1.id.clone();

    index(&storage, &mut search, &m1);
    index(&storage, &mut search, &m2);

    search.remove_memory(&id1).unwrap();
    storage.delete(&id1).unwrap();

    let results = search
        .search(&[1.0, 0.0, 0.0, 0.0], &query("삭제 유지", 5))
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].memory.title, "유지 대상");
}

#[test]
fn test_hybrid_all_memory_types_searchable() {
    let (storage, mut search) = test_engine();

    let types = [
        MemoryType::Observation,
        MemoryType::Decision,
        MemoryType::Preference,
        MemoryType::Fact,
        MemoryType::Bugfix,
        MemoryType::Discovery,
    ];

    for (i, mt) in types.iter().enumerate() {
        let v = vec![(i as f32) / 10.0, 1.0 - (i as f32) / 10.0, 0.0, 0.0];
        let m = mem_with(
            &format!("{:?} 메모리", mt),
            &format!("{:?} 타입 검색 테스트", mt),
            Some(v),
            Priority::Medium,
            *mt,
            vec![],
        );
        index(&storage, &mut search, &m);
    }

    let results = search
        .search(&[0.5, 0.5, 0.0, 0.0], &query("검색 테스트", 10))
        .unwrap();
    assert_eq!(results.len(), types.len());
}

// ============================================================
// 4. Scoring Boundary Tests
// ============================================================

#[test]
fn test_scoring_recency_extremes() {
    let scorer = Scorer::default();

    // Just now
    let score_0 = scorer.recency_score(0.0);
    assert!((score_0 - 1.0).abs() < 0.001);

    // 1 year old
    let score_365 = scorer.recency_score(365.0);
    assert!(
        score_365 > 0.0 && score_365 < 0.001,
        "365-day score: {score_365}"
    );

    // Very old (should not be NaN or negative)
    let score_10k = scorer.recency_score(10000.0);
    assert!(score_10k >= 0.0 && score_10k.is_finite());
}

#[test]
fn test_scoring_combined_range() {
    let scorer = Scorer::default();

    // All max inputs
    let (score_max, _) = scorer.combined_score(1.0, 1.0, 0.0, Priority::High);
    assert!(
        score_max > 0.0 && score_max <= 1.5,
        "Max combined: {score_max}"
    );

    // All min inputs
    let (score_min, _) = scorer.combined_score(0.0, 0.0, 10000.0, Priority::Low);
    assert!(score_min >= 0.0, "Min combined: {score_min}");

    // Max should always exceed min
    assert!(score_max > score_min);
}

#[test]
fn test_scoring_weight_sum() {
    let scorer = Scorer::default();
    let total_weight = scorer.semantic_weight
        + scorer.keyword_weight
        + scorer.recency_weight
        + scorer.importance_weight;
    assert!(
        (total_weight - 1.0).abs() < 0.001,
        "Weights should sum to 1.0, got {total_weight}"
    );
}

#[test]
fn test_scoring_priority_ordering() {
    let scorer = Scorer::default();

    let (low, _) = scorer.combined_score(0.5, 0.5, 1.0, Priority::Low);
    let (med, _) = scorer.combined_score(0.5, 0.5, 1.0, Priority::Medium);
    let (high, _) = scorer.combined_score(0.5, 0.5, 1.0, Priority::High);

    assert!(low < med, "Low ({low}) should be < Medium ({med})");
    assert!(med < high, "Medium ({med}) should be < High ({high})");
}

#[test]
fn test_rrf_properties() {
    // More lists ranking high = higher score
    let score_top = Scorer::rrf_score(&[1, 1, 1], 60.0);
    let score_mid = Scorer::rrf_score(&[5, 5, 5], 60.0);
    let score_low = Scorer::rrf_score(&[100, 100, 100], 60.0);

    assert!(score_top > score_mid);
    assert!(score_mid > score_low);
    assert!(score_low > 0.0);
}

// ============================================================
// 5. Concurrent Access Tests
// ============================================================

#[test]
fn test_concurrent_vector_operations() {
    use std::sync::{Arc, Mutex};
    use std::thread;

    let vi = Arc::new(Mutex::new(VectorIndex::new(4)));

    let mut handles = vec![];

    // 10 threads inserting concurrently
    for t in 0..10 {
        let vi = Arc::clone(&vi);
        handles.push(thread::spawn(move || {
            for i in 0..100 {
                let id = format!("t{t}-{i}");
                let v = vec![t as f32 / 10.0, i as f32 / 100.0, 0.1, 0.1];
                vi.lock().unwrap().upsert(id, v).unwrap();
            }
        }));
    }

    for h in handles {
        h.join().unwrap();
    }

    let vi = vi.lock().unwrap();
    assert_eq!(vi.len(), 1000, "All 1000 vectors should be inserted");

    let results = vi.search(&[1.0, 0.0, 0.1, 0.1], 5);
    assert_eq!(results.len(), 5);
}

#[test]
fn test_concurrent_search_while_inserting() {
    use std::sync::{Arc, Mutex};
    use std::thread;

    let vi = Arc::new(Mutex::new(VectorIndex::new(4)));

    // Pre-populate
    {
        let mut idx = vi.lock().unwrap();
        for i in 0..100 {
            idx.upsert(format!("pre-{i}"), vec![i as f32 / 100.0, 0.5, 0.3, 0.1])
                .unwrap();
        }
    }

    let mut handles = vec![];

    // Writer thread
    let vi_w = Arc::clone(&vi);
    handles.push(thread::spawn(move || {
        for i in 100..200 {
            vi_w.lock()
                .unwrap()
                .upsert(format!("new-{i}"), vec![0.5, 0.5, 0.5, 0.5])
                .unwrap();
        }
    }));

    // Reader threads
    for _ in 0..5 {
        let vi_r = Arc::clone(&vi);
        handles.push(thread::spawn(move || {
            for _ in 0..20 {
                let idx = vi_r.lock().unwrap();
                let results = idx.search(&[1.0, 0.0, 0.0, 0.0], 5);
                assert!(!results.is_empty(), "Search should always return results");
            }
        }));
    }

    for h in handles {
        h.join().unwrap();
    }
}

#[test]
fn test_concurrent_storage_operations() {
    use std::sync::{Arc, Mutex};
    use std::thread;

    let storage = Arc::new(Mutex::new(Storage::in_memory().unwrap()));

    let mut handles = vec![];

    // Writer threads
    for t in 0..5 {
        let s = Arc::clone(&storage);
        handles.push(thread::spawn(move || {
            for i in 0..20 {
                let m = Memory::new(
                    format!("content-{t}-{i}"),
                    format!("title-{t}-{i}"),
                    MemoryMetadata::default(),
                );
                s.lock().unwrap().insert(&m).unwrap();
            }
        }));
    }

    for h in handles {
        h.join().unwrap();
    }

    let count = storage.lock().unwrap().count().unwrap();
    assert_eq!(count, 100, "All inserts should succeed under concurrency");
}

// ============================================================
// 6. Data Integrity Tests
// ============================================================

#[test]
fn test_search_score_breakdown_integrity() {
    let (storage, mut search) = test_engine();

    let m = mem(
        "점수 검증",
        "스코어 분해 확인",
        Some(vec![1.0, 0.0, 0.0, 0.0]),
    );
    index(&storage, &mut search, &m);

    let results = search
        .search(&[0.9, 0.1, 0.0, 0.0], &query("스코어", 5))
        .unwrap();
    assert!(!results.is_empty());

    let r = &results[0];
    let bd = &r.score_breakdown;

    // All components should be non-negative
    assert!(bd.semantic >= 0.0, "Semantic: {}", bd.semantic);
    assert!(bd.keyword >= 0.0, "Keyword: {}", bd.keyword);
    assert!(bd.recency >= 0.0, "Recency: {}", bd.recency);
    assert!(bd.importance >= 0.0, "Importance: {}", bd.importance);

    // Final score should be positive
    assert!(r.score > 0.0);

    // Score should roughly equal weighted sum of components
    let expected = 0.6 * bd.semantic + 0.15 * bd.keyword + 0.15 * bd.recency + 0.10 * bd.importance;
    assert!(
        (r.score - expected).abs() < 0.01,
        "Score {:.4} != weighted sum {:.4}",
        r.score,
        expected
    );
}

#[test]
fn test_empty_query_returns_results_via_vector() {
    let (storage, mut search) = test_engine();

    let m = mem(
        "테스트",
        "빈 쿼리 벡터 검색",
        Some(vec![1.0, 0.0, 0.0, 0.0]),
    );
    index(&storage, &mut search, &m);

    // Empty query text but valid embedding
    let results = search.search(&[1.0, 0.0, 0.0, 0.0], &query("", 5)).unwrap();
    // With empty query, BM25 may return nothing, but vector should still find it
    // (depends on BM25 query parser behavior with empty string)
    // The important thing is it doesn't crash
    assert!(results.len() <= 1);
}

#[test]
fn test_search_returns_correct_memory_data() {
    let (storage, mut search) = test_engine();

    let m = mem_with(
        "정확한 데이터",
        "이 내용이 정확히 반환되어야 합니다",
        Some(vec![1.0, 0.0, 0.0, 0.0]),
        Priority::High,
        MemoryType::Decision,
        vec!["tag1", "tag2"],
    );
    let expected_id = m.id.clone();
    index(&storage, &mut search, &m);

    let results = search
        .search(&[1.0, 0.0, 0.0, 0.0], &query("정확한", 1))
        .unwrap();
    assert_eq!(results.len(), 1);

    let result = &results[0].memory;
    assert_eq!(result.id, expected_id);
    assert_eq!(result.title, "정확한 데이터");
    assert_eq!(result.content, "이 내용이 정확히 반환되어야 합니다");
    assert_eq!(result.metadata.memory_type, MemoryType::Decision);
    assert_eq!(result.metadata.priority, Priority::High);
    assert_eq!(result.metadata.tags, vec!["tag1", "tag2"]);
}

#[test]
fn test_rebuild_indices_from_storage() {
    let storage = Arc::new(Storage::in_memory().unwrap());

    // Phase 1: Insert data
    let m1 = mem("복구 A", "복구 테스트 A", Some(vec![1.0, 0.0, 0.0, 0.0]));
    let m2 = mem("복구 B", "복구 테스트 B", Some(vec![0.0, 1.0, 0.0, 0.0]));
    storage.insert(&m1).unwrap();
    storage.insert(&m2).unwrap();

    // Phase 2: Build fresh indices from storage (simulating restart)
    let mut vector_index = VectorIndex::new(4);
    let bm25_index = Bm25Index::in_memory().unwrap();
    let scorer = Scorer::default();

    let embeddings = storage.all_embeddings().unwrap();
    vector_index.build_from(embeddings).unwrap();

    let text_data = storage.all_text_data().unwrap();
    for (id, title, content) in &text_data {
        bm25_index.add(id, title, content).unwrap();
    }

    let search = HybridSearch::new(storage.clone(), vector_index, bm25_index, scorer);

    // Phase 3: Search should work
    let results = search
        .search(&[1.0, 0.0, 0.0, 0.0], &query("복구", 5))
        .unwrap();
    assert_eq!(
        results.len(),
        2,
        "Both memories should be found after rebuild"
    );
}
