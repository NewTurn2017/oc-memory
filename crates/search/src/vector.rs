use anyhow::{Context, Result};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use usearch::{Index, IndexOptions, MetricKind, ScalarKind};

/// In-process HNSW vector index backed by usearch.
///
/// Provides O(log n) approximate nearest-neighbor search instead of
/// brute-force O(n) scanning. The public API is identical to the
/// previous brute-force implementation so callers need no changes.
pub struct VectorIndex {
    index: Index,
    dimensions: usize,
    /// Forward map: String UUID → u64 key
    id_to_key: HashMap<String, u64>,
    /// Reverse map: u64 key → String UUID
    key_to_id: HashMap<u64, String>,
    /// Monotonically increasing key generator
    next_key: AtomicU64,
}

impl VectorIndex {
    pub fn new(dimensions: usize) -> Self {
        let options = IndexOptions {
            dimensions,
            metric: MetricKind::Cos,
            quantization: ScalarKind::F32,
            connectivity: 0,     // auto
            expansion_add: 0,    // auto
            expansion_search: 0, // auto
            multi: false,
        };
        let index = Index::new(&options).expect("Failed to create usearch index");
        // Reserve a reasonable initial capacity
        index
            .reserve(1024)
            .expect("Failed to reserve usearch capacity");

        Self {
            index,
            dimensions,
            id_to_key: HashMap::new(),
            key_to_id: HashMap::new(),
            next_key: AtomicU64::new(1),
        }
    }

    /// Allocate a new u64 key for a string ID.
    fn alloc_key(&self) -> u64 {
        self.next_key.fetch_add(1, Ordering::Relaxed)
    }

    /// Ensure the index has capacity for at least one more vector.
    fn ensure_capacity(&self) -> Result<()> {
        let current_size = self.index.size();
        let current_capacity = self.index.capacity();
        if current_size >= current_capacity {
            let new_capacity = (current_capacity * 2).max(1024);
            self.index
                .reserve(new_capacity)
                .context("Failed to grow usearch index capacity")?;
        }
        Ok(())
    }

    /// Add or update a vector.
    pub fn upsert(&mut self, id: String, vector: Vec<f32>) -> Result<()> {
        anyhow::ensure!(
            vector.len() == self.dimensions,
            "Vector dimension mismatch: expected {}, got {}",
            self.dimensions,
            vector.len()
        );

        // If the id already exists, remove the old entry first.
        if let Some(&old_key) = self.id_to_key.get(&id) {
            let _ = self.index.remove(old_key);
        }

        let key = if let Some(&existing) = self.id_to_key.get(&id) {
            existing
        } else {
            let k = self.alloc_key();
            self.id_to_key.insert(id.clone(), k);
            self.key_to_id.insert(k, id);
            k
        };

        self.ensure_capacity()?;
        self.index.add(key, &vector).context("usearch add failed")?;
        Ok(())
    }

    /// Remove a vector. Returns true if it existed.
    pub fn remove(&mut self, id: &str) -> bool {
        if let Some(key) = self.id_to_key.remove(id) {
            self.key_to_id.remove(&key);
            let _ = self.index.remove(key);
            true
        } else {
            false
        }
    }

    /// Search for nearest neighbors. Returns `(id, similarity)` pairs
    /// sorted by descending cosine similarity.
    ///
    /// usearch's Cos metric returns **distance** = 1 − cos_sim,
    /// so we convert: `similarity = 1.0 − distance`.
    pub fn search(&self, query: &[f32], limit: usize) -> Vec<(String, f32)> {
        if self.index.size() == 0 || limit == 0 {
            return Vec::new();
        }

        let actual_limit = limit.min(self.index.size());

        let matches = match self.index.search(query, actual_limit) {
            Ok(m) => m,
            Err(_) => return Vec::new(),
        };

        let mut results: Vec<(String, f32)> = matches
            .keys
            .iter()
            .zip(matches.distances.iter())
            .filter_map(|(&key, &distance)| {
                self.key_to_id.get(&key).map(|id| {
                    // Convert distance to similarity. Clamp to [0, 1].
                    let similarity = (1.0 - distance).clamp(0.0, 1.0);
                    (id.clone(), similarity)
                })
            })
            .collect();

        // usearch returns results sorted by distance (ascending),
        // which means similarity is already descending, but we sort
        // explicitly to be safe.
        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(limit);
        results
    }

    /// Number of vectors in the index.
    pub fn len(&self) -> usize {
        self.id_to_key.len()
    }

    pub fn is_empty(&self) -> bool {
        self.id_to_key.is_empty()
    }

    /// Rebuild index from a batch of entries.
    pub fn build_from(&mut self, entries: Vec<(String, Vec<f32>)>) -> Result<()> {
        // Reset everything
        self.id_to_key.clear();
        self.key_to_id.clear();
        self.next_key.store(1, Ordering::Relaxed);

        // Create a fresh index
        let options = IndexOptions {
            dimensions: self.dimensions,
            metric: MetricKind::Cos,
            quantization: ScalarKind::F32,
            connectivity: 0,
            expansion_add: 0,
            expansion_search: 0,
            multi: false,
        };
        self.index = Index::new(&options).context("Failed to recreate usearch index")?;

        let capacity = entries.len().max(1024);
        self.index
            .reserve(capacity)
            .context("Failed to reserve capacity for build_from")?;

        for (id, vec) in entries {
            self.upsert(id, vec)?;
        }
        tracing::info!(
            count = self.id_to_key.len(),
            "Vector index built with usearch HNSW"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vector_index_upsert_and_search() {
        let mut index = VectorIndex::new(3);
        index.upsert("a".to_string(), vec![1.0, 0.0, 0.0]).unwrap();
        index.upsert("b".to_string(), vec![0.9, 0.1, 0.0]).unwrap();
        index.upsert("c".to_string(), vec![0.0, 1.0, 0.0]).unwrap();

        let results = index.search(&[1.0, 0.0, 0.0], 2);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, "a");
        // "a" should have similarity ~1.0
        assert!(
            results[0].1 > 0.95,
            "Expected high similarity for identical vector, got {}",
            results[0].1
        );
    }

    #[test]
    fn test_vector_index_remove() {
        let mut index = VectorIndex::new(3);
        index.upsert("a".to_string(), vec![1.0, 0.0, 0.0]).unwrap();
        assert_eq!(index.len(), 1);

        let removed = index.remove("a");
        assert!(removed);
        assert_eq!(index.len(), 0);

        let removed_again = index.remove("a");
        assert!(!removed_again);
    }

    #[test]
    fn test_vector_index_build_from() {
        let mut index = VectorIndex::new(3);
        let entries = vec![
            ("x".to_string(), vec![1.0, 0.0, 0.0]),
            ("y".to_string(), vec![0.0, 1.0, 0.0]),
            ("z".to_string(), vec![0.0, 0.0, 1.0]),
        ];
        index.build_from(entries).unwrap();
        assert_eq!(index.len(), 3);

        let results = index.search(&[1.0, 0.0, 0.0], 1);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "x");
    }

    #[test]
    fn test_vector_index_upsert_overwrites() {
        let mut index = VectorIndex::new(3);
        index.upsert("a".to_string(), vec![1.0, 0.0, 0.0]).unwrap();
        // Overwrite with different vector
        index.upsert("a".to_string(), vec![0.0, 1.0, 0.0]).unwrap();
        assert_eq!(index.len(), 1);

        // Search should find the updated vector
        let results = index.search(&[0.0, 1.0, 0.0], 1);
        assert_eq!(results[0].0, "a");
        assert!(
            results[0].1 > 0.95,
            "Expected high similarity after upsert overwrite"
        );
    }

    #[test]
    fn test_vector_index_empty_search() {
        let index = VectorIndex::new(3);
        let results = index.search(&[1.0, 0.0, 0.0], 5);
        assert!(results.is_empty());
    }

    #[test]
    fn test_dimension_mismatch() {
        let mut index = VectorIndex::new(3);
        let result = index.upsert("a".to_string(), vec![1.0, 0.0]);
        assert!(result.is_err(), "Should reject wrong dimensions");
    }
}
