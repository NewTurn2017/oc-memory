use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use chrono::Utc;
use oc_core::Storage;
use oc_core::models::{Memory, SearchQuery, SearchResult};

use crate::bm25::Bm25Index;
use crate::scoring::Scorer;
use crate::vector::VectorIndex;

/// Hybrid search combining vector similarity + BM25 keyword search + time decay
pub struct HybridSearch {
    storage: Arc<Storage>,
    vector_index: VectorIndex,
    bm25_index: Bm25Index,
    scorer: Scorer,
}

impl HybridSearch {
    pub fn new(
        storage: Arc<Storage>,
        vector_index: VectorIndex,
        bm25_index: Bm25Index,
        scorer: Scorer,
    ) -> Self {
        Self {
            storage,
            vector_index,
            bm25_index,
            scorer,
        }
    }

    /// Mutable access to vector index (for loading embeddings)
    pub fn vector_index_mut(&mut self) -> &mut VectorIndex {
        &mut self.vector_index
    }

    /// Search memories using hybrid vector + BM25 with RRF fusion
    pub fn search(
        &self,
        query_embedding: &[f32],
        query: &SearchQuery,
    ) -> Result<Vec<SearchResult>> {
        let expanded_limit = query.limit * 3; // Over-fetch for fusion

        // 1. Vector search
        let vector_results = self.vector_index.search(query_embedding, expanded_limit);

        // 2. BM25 keyword search
        let bm25_results = self
            .bm25_index
            .search(&query.query, expanded_limit)
            .unwrap_or_default();

        // 3. Build rank maps
        let mut vector_ranks: HashMap<String, usize> = HashMap::new();
        let mut vector_scores: HashMap<String, f32> = HashMap::new();
        for (rank, (id, score)) in vector_results.iter().enumerate() {
            vector_ranks.insert(id.clone(), rank + 1);
            vector_scores.insert(id.clone(), *score);
        }

        let mut bm25_ranks: HashMap<String, usize> = HashMap::new();
        let mut bm25_scores: HashMap<String, f32> = HashMap::new();
        for (rank, (id, score)) in bm25_results.iter().enumerate() {
            bm25_ranks.insert(id.clone(), rank + 1);
            bm25_scores.insert(id.clone(), *score);
        }

        // 4. Collect all candidate IDs
        let mut all_ids: Vec<String> = vector_ranks
            .keys()
            .chain(bm25_ranks.keys())
            .cloned()
            .collect();
        all_ids.sort();
        all_ids.dedup();

        // 5. Score each candidate
        let now = Utc::now();
        let mut scored_results: Vec<(String, f32, oc_core::models::ScoreBreakdown)> = Vec::new();

        for id in &all_ids {
            let semantic = *vector_scores.get(id).unwrap_or(&0.0);

            // Normalize BM25 score to [0, 1]
            let max_bm25 = bm25_scores
                .values()
                .cloned()
                .fold(f32::NEG_INFINITY, f32::max);
            let keyword = if max_bm25 > 0.0 {
                bm25_scores.get(id).unwrap_or(&0.0) / max_bm25
            } else {
                0.0
            };

            // Get memory for time/priority info
            if let Ok(Some(memory)) = self.storage.get(id) {
                let days_since = (now - memory.accessed_at).num_hours() as f32 / 24.0;
                let (score, breakdown) = self.scorer.combined_score(
                    semantic,
                    keyword,
                    days_since,
                    memory.metadata.priority,
                );
                scored_results.push((id.clone(), score, breakdown));
            }
        }

        // 6. Sort by final score
        scored_results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored_results.truncate(query.limit);

        // 7. Fetch full memories and build results
        let result_ids: Vec<String> = scored_results.iter().map(|(id, _, _)| id.clone()).collect();
        let memories = self.storage.get_many(&result_ids)?;

        let memory_map: HashMap<String, Memory> =
            memories.into_iter().map(|m| (m.id.clone(), m)).collect();

        let results = scored_results
            .into_iter()
            .filter_map(|(id, score, breakdown)| {
                memory_map.get(&id).map(|memory| {
                    // Touch for access tracking
                    let _ = self.storage.touch(&id);

                    SearchResult {
                        memory: if query.index_only {
                            // Strip content for token savings
                            Memory {
                                content: String::new(),
                                embedding: None,
                                ..memory.clone()
                            }
                        } else {
                            Memory {
                                embedding: None,
                                ..memory.clone()
                            }
                        },
                        score,
                        score_breakdown: breakdown,
                    }
                })
            })
            .collect();

        Ok(results)
    }

    /// Add a memory to both indices
    pub fn index_memory(&mut self, memory: &Memory) -> Result<()> {
        // Add to vector index
        if let Some(ref embedding) = memory.embedding {
            self.vector_index
                .upsert(memory.id.clone(), embedding.clone())?;
        }

        // Add to BM25 index
        self.bm25_index
            .add(&memory.id, &memory.title, &memory.content)?;

        Ok(())
    }

    /// Add text only to BM25 index (for rebuilding without full Memory object)
    pub fn index_memory_text(&mut self, id: &str, title: &str, content: &str) -> Result<()> {
        self.bm25_index.add(id, title, content)?;
        Ok(())
    }

    /// Remove a memory from both indices
    pub fn remove_memory(&mut self, id: &str) -> Result<()> {
        self.vector_index.remove(id);
        self.bm25_index.remove(id)?;
        Ok(())
    }

    /// Number of indexed memories
    pub fn indexed_count(&self) -> usize {
        self.vector_index.len()
    }
}
