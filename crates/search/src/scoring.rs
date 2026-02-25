use oc_core::models::{Priority, ScoreBreakdown};

/// Combined scoring with time decay, importance weighting, and RRF fusion
pub struct Scorer {
    pub semantic_weight: f32,
    pub keyword_weight: f32,
    pub recency_weight: f32,
    pub importance_weight: f32,
    /// Half-life in days for recency decay
    pub half_life_days: f32,
}

impl Scorer {
    /// Calculate recency score using exponential decay
    ///
    /// score = exp(-λ * days_since_access)
    /// where λ = ln(2) / half_life_days
    pub fn recency_score(&self, days_since_access: f32) -> f32 {
        let lambda = (2.0_f32).ln() / self.half_life_days;
        (-lambda * days_since_access).exp()
    }

    /// Calculate importance score from priority
    pub fn importance_score(&self, priority: Priority) -> f32 {
        priority.weight()
    }

    /// Combine all scores into a final ranking score
    pub fn combined_score(
        &self,
        semantic: f32,
        keyword: f32,
        days_since_access: f32,
        priority: Priority,
    ) -> (f32, ScoreBreakdown) {
        let recency = self.recency_score(days_since_access);
        let importance = self.importance_score(priority);

        let score = self.semantic_weight * semantic
            + self.keyword_weight * keyword
            + self.recency_weight * recency
            + self.importance_weight * importance;

        let breakdown = ScoreBreakdown {
            semantic,
            keyword,
            recency,
            importance,
        };

        (score, breakdown)
    }

    /// Reciprocal Rank Fusion: combine vector and BM25 rankings
    ///
    /// RRF(d) = Σ 1 / (k + rank_i(d))
    /// k = 60 is standard
    pub fn rrf_score(ranks: &[usize], k: f32) -> f32 {
        ranks.iter().map(|&rank| 1.0 / (k + rank as f32)).sum()
    }
}

impl Default for Scorer {
    fn default() -> Self {
        Self {
            semantic_weight: 0.6,
            keyword_weight: 0.15,
            recency_weight: 0.15,
            importance_weight: 0.10,
            half_life_days: 30.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_recency_decay() {
        let scorer = Scorer::default();

        // At day 0, score should be 1.0
        let score_0 = scorer.recency_score(0.0);
        assert!((score_0 - 1.0).abs() < 0.001);

        // At half-life, score should be 0.5
        let score_half = scorer.recency_score(30.0);
        assert!((score_half - 0.5).abs() < 0.01);

        // At 60 days, score should be ~0.25
        let score_60 = scorer.recency_score(60.0);
        assert!((score_60 - 0.25).abs() < 0.01);
    }

    #[test]
    fn test_rrf() {
        // Item ranked 1st in both lists
        let score = Scorer::rrf_score(&[1, 1], 60.0);
        assert!((score - 2.0 / 61.0).abs() < 0.001);
    }
}
