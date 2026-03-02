//! Temporal decay and composite ranking for memory retrieval.

use chrono::{DateTime, Utc};

use trusty_models::memory::Memory;

/// A memory paired with its computed relevance score.
#[derive(Debug, Clone)]
pub struct RankedMemory {
    pub memory: Memory,
    pub score: f32,
    pub strength: f32,
}

/// Compute the current strength of a memory given its age.
///
/// Formula: `strength = exp(-decay_rate * days_since_creation * (1 - importance * 0.5))`
/// Where `decay_rate = ln(2) / half_life_days` (category-dependent).
pub fn compute_strength(memory: &Memory, now: DateTime<Utc>) -> f32 {
    let half_life = memory.category.decay_half_life_days();
    let decay_rate = std::f32::consts::LN_2 / half_life;
    let days_elapsed = (now - memory.created_at).num_days() as f32;
    let days_elapsed = days_elapsed.max(0.0);
    let effective_rate = decay_rate * (1.0 - memory.importance * 0.5);
    (-effective_rate * days_elapsed).exp()
}

/// Compute the composite ranking score for retrieval ordering.
///
/// Formula: `score = strength * 0.5 + confidence_proxy * 0.3 + importance * 0.2`
/// Where `confidence_proxy = 1.0 / (1.0 + distance)` from vector search.
pub fn ranking_score(strength: f32, distance: f32, importance: f32) -> f32 {
    let confidence_proxy = 1.0 / (1.0 + distance);
    strength * 0.5 + confidence_proxy * 0.3 + importance * 0.2
}

/// Rank a list of `(Memory, distance)` pairs by composite score (descending).
pub fn rank_memories(pairs: Vec<(Memory, f32)>, now: DateTime<Utc>) -> Vec<RankedMemory> {
    let mut ranked: Vec<RankedMemory> = pairs
        .into_iter()
        .map(|(memory, distance)| {
            let strength = compute_strength(&memory, now);
            let score = ranking_score(strength, distance, memory.importance);
            RankedMemory {
                memory,
                score,
                strength,
            }
        })
        .collect();
    ranked.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    ranked
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use trusty_models::memory::{Memory, MemoryCategory};
    use uuid::Uuid;

    fn make_memory(category: MemoryCategory, importance: f32, created_at: DateTime<Utc>) -> Memory {
        let now = Utc::now();
        Memory {
            id: Uuid::new_v4(),
            user_id: "test".to_string(),
            category,
            content: "test memory".to_string(),
            embedding: None,
            related_entities: vec![],
            source_id: None,
            importance,
            access_count: 0,
            last_accessed: None,
            created_at,
            updated_at: now,
        }
    }

    #[test]
    fn test_fresh_memory_has_high_strength() {
        let memory = make_memory(MemoryCategory::UserPreference, 0.5, Utc::now());
        let strength = compute_strength(&memory, Utc::now());
        // A memory created right now should have strength ≈ 1.0
        assert!(strength > 0.99, "Expected strength ≈ 1.0, got {strength}");
    }

    #[test]
    fn test_old_memory_has_lower_strength() {
        // UserPreference half_life = 365 days, importance = 0.0 → effective_rate = ln(2)/365
        // After 365 days: strength = exp(-ln(2)/365 * 365 * 1.0) = exp(-ln(2)) = 0.5
        let created_at = Utc::now() - Duration::days(365);
        let memory = make_memory(MemoryCategory::UserPreference, 0.0, created_at);
        let strength = compute_strength(&memory, Utc::now());
        // Should be ≈ 0.5 (one half-life with importance=0 gives no modulation)
        assert!(
            (strength - 0.5).abs() < 0.01,
            "Expected strength ≈ 0.5, got {strength}"
        );
    }

    #[test]
    fn test_high_importance_slows_decay() {
        let created_at = Utc::now() - Duration::days(90);
        let high_importance = make_memory(MemoryCategory::ProjectFact, 1.0, created_at);
        let low_importance = make_memory(MemoryCategory::ProjectFact, 0.0, created_at);
        let now = Utc::now();
        let high_strength = compute_strength(&high_importance, now);
        let low_strength = compute_strength(&low_importance, now);
        assert!(
            high_strength > low_strength,
            "High importance should decay slower: {high_strength} vs {low_strength}"
        );
    }

    #[test]
    fn test_ranking_score_range() {
        // Score components: strength in [0,1], confidence_proxy in (0,1], importance in [0,1]
        // Maximum: 1.0*0.5 + 1.0*0.3 + 1.0*0.2 = 1.0
        // Minimum: approaches 0.0 for fully decayed, distant, unimportant memory
        let score = ranking_score(0.8, 0.1, 0.6);
        assert!(score >= 0.0, "Score should be non-negative");
        assert!(score <= 1.0, "Score should be at most 1.0");

        let max_score = ranking_score(1.0, 0.0, 1.0);
        assert!(
            max_score <= 1.0,
            "Max score should be ≤ 1.0, got {max_score}"
        );
    }

    #[test]
    fn test_rank_memories_sorted_descending() {
        let now = Utc::now();
        // Fresh memory with high importance
        let fresh = make_memory(MemoryCategory::UserPreference, 1.0, now);
        // Old memory with low importance
        let old = make_memory(MemoryCategory::General, 0.0, now - Duration::days(200));
        let pairs = vec![(fresh, 0.1), (old, 0.5)];
        let ranked = rank_memories(pairs, now);
        assert_eq!(ranked.len(), 2, "Should return 2 ranked memories");
        assert!(
            ranked[0].score >= ranked[1].score,
            "Results should be sorted descending: {} >= {}",
            ranked[0].score,
            ranked[1].score
        );
    }
}
