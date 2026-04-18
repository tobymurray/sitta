//! Individual animal matcher using cosine similarity on Perch embeddings.

use std::collections::HashMap;
use std::sync::Arc;

use arc_swap::ArcSwap;
use uuid::Uuid;

use crate::db::Database;
use crate::models::uuid_from_blob;
use crate::StoreError;

/// Result of matching an embedding against known individuals.
#[derive(Debug, Clone)]
pub struct MatchResult {
    pub individual_id: Uuid,
    pub individual_label: String,
    pub similarity: f32,
}

/// In-memory cache of reference embeddings, grouped by species.
/// Compares new embeddings against enrolled individuals using cosine similarity.
pub struct IndividualMatcher {
    entries: ArcSwap<HashMap<String, Vec<ReferenceEntry>>>,
    db: Database,
    threshold: f32,
}

struct ReferenceEntry {
    individual_id: Uuid,
    label: String,
    embedding: Vec<f32>,
}

impl IndividualMatcher {
    /// Create a matcher, loading all reference embeddings from the database.
    pub async fn new(db: Database, threshold: f32) -> Result<Self, StoreError> {
        let entries = load_entries(&db).await?;
        Ok(Self {
            entries: ArcSwap::from_pointee(entries),
            db,
            threshold,
        })
    }

    /// Find the best-matching individual for an embedding within a species.
    /// Returns `None` if no match exceeds the threshold.
    pub fn find_match(&self, scientific_name: &str, embedding: &[f32]) -> Option<MatchResult> {
        let map = self.entries.load();
        let candidates = map.get(scientific_name)?;

        let mut best: Option<MatchResult> = None;
        for entry in candidates {
            if entry.embedding.len() != embedding.len() {
                tracing::warn!(
                    individual = %entry.individual_id,
                    expected = embedding.len(),
                    got = entry.embedding.len(),
                    "Embedding dimension mismatch, skipping"
                );
                continue;
            }
            let sim = cosine_similarity(embedding, &entry.embedding);
            if sim >= self.threshold
                && best.as_ref().is_none_or(|b| sim > b.similarity)
            {
                best = Some(MatchResult {
                    individual_id: entry.individual_id,
                    individual_label: entry.label.clone(),
                    similarity: sim,
                });
            }
        }
        best
    }

    /// Count enrolled individuals for a species (for auto-labeling).
    pub fn count_for_species(&self, scientific_name: &str) -> usize {
        let map = self.entries.load();
        map.get(scientific_name).map_or(0, |v| v.len())
    }

    /// Reload all reference embeddings from the database. Call after enrollment
    /// or embedding updates.
    pub async fn reload(&self) -> Result<(), StoreError> {
        let entries = load_entries(&self.db).await?;
        self.entries.store(Arc::new(entries));
        Ok(())
    }
}

async fn load_entries(db: &Database) -> Result<HashMap<String, Vec<ReferenceEntry>>, StoreError> {
    let rows = db.load_reference_embeddings().await?;
    let mut map: HashMap<String, Vec<ReferenceEntry>> = HashMap::new();
    for row in rows {
        let id = match uuid_from_blob(row.id) {
            Ok(id) => id,
            Err(_) => continue,
        };
        let Some(blob) = row.reference_embedding else {
            continue;
        };
        if blob.len() % 4 != 0 {
            tracing::warn!(individual = %id, len = blob.len(), "Invalid embedding blob size");
            continue;
        }
        let embedding: Vec<f32> = bytemuck::cast_slice::<u8, f32>(&blob).to_vec();
        map.entry(row.scientific_name.clone())
            .or_default()
            .push(ReferenceEntry {
                individual_id: id,
                label: row.label,
                embedding,
            });
    }
    Ok(map)
}

/// Cosine similarity between two f32 vectors.
/// Uses f64 accumulators to avoid precision loss over 1536+ dimensions.
/// Returns 0.0 if either vector has zero norm.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());
    let mut dot = 0.0_f64;
    let mut norm_a = 0.0_f64;
    let mut norm_b = 0.0_f64;
    for (x, y) in a.iter().zip(b.iter()) {
        let x = *x as f64;
        let y = *y as f64;
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }
    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom == 0.0 {
        return 0.0;
    }
    (dot / denom) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_identical_vectors() {
        let v = vec![1.0, 2.0, 3.0];
        let sim = cosine_similarity(&v, &v);
        assert!((sim - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_orthogonal_vectors() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!(sim.abs() < 1e-6);
    }

    #[test]
    fn cosine_opposite_vectors() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![-1.0, -2.0, -3.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim - (-1.0)).abs() < 1e-6);
    }

    #[test]
    fn cosine_zero_vector() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![0.0, 0.0, 0.0];
        assert_eq!(cosine_similarity(&a, &b), 0.0);
        assert_eq!(cosine_similarity(&b, &a), 0.0);
        assert_eq!(cosine_similarity(&b, &b), 0.0);
    }

    #[test]
    fn cosine_high_dimensional() {
        // Simulate 1536-dim vectors.
        let a: Vec<f32> = (0..1536).map(|i| (i as f32).sin()).collect();
        let b = a.clone();
        let sim = cosine_similarity(&a, &b);
        assert!((sim - 1.0).abs() < 1e-5);
    }

    #[test]
    fn find_match_no_candidates() {
        let matcher = IndividualMatcher {
            entries: ArcSwap::from_pointee(HashMap::new()),
            db: unreachable_db(),
            threshold: 0.85,
        };
        assert!(matcher.find_match("Tyto alba", &[1.0, 2.0, 3.0]).is_none());
    }

    #[test]
    fn find_match_below_threshold() {
        let mut map = HashMap::new();
        map.insert(
            "Tyto alba".to_string(),
            vec![ReferenceEntry {
                individual_id: Uuid::nil(),
                label: "Owl #1".into(),
                embedding: vec![1.0, 0.0, 0.0],
            }],
        );
        let matcher = IndividualMatcher {
            entries: ArcSwap::from_pointee(map),
            db: unreachable_db(),
            threshold: 0.99,
        };
        // Orthogonal-ish vector: low similarity.
        assert!(matcher.find_match("Tyto alba", &[0.1, 1.0, 0.0]).is_none());
    }

    #[test]
    fn find_match_above_threshold() {
        let mut map = HashMap::new();
        map.insert(
            "Tyto alba".to_string(),
            vec![ReferenceEntry {
                individual_id: Uuid::nil(),
                label: "Owl #1".into(),
                embedding: vec![1.0, 2.0, 3.0],
            }],
        );
        let matcher = IndividualMatcher {
            entries: ArcSwap::from_pointee(map),
            db: unreachable_db(),
            threshold: 0.5,
        };
        let result = matcher.find_match("Tyto alba", &[1.0, 2.0, 3.0]);
        assert!(result.is_some());
        let m = result.unwrap();
        assert!((m.similarity - 1.0).abs() < 1e-6);
        assert_eq!(m.individual_label, "Owl #1");
    }

    #[test]
    fn find_match_wrong_species_no_match() {
        let mut map = HashMap::new();
        map.insert(
            "Tyto alba".to_string(),
            vec![ReferenceEntry {
                individual_id: Uuid::nil(),
                label: "Owl #1".into(),
                embedding: vec![1.0, 2.0, 3.0],
            }],
        );
        let matcher = IndividualMatcher {
            entries: ArcSwap::from_pointee(map),
            db: unreachable_db(),
            threshold: 0.5,
        };
        // Different species — should not match.
        assert!(matcher.find_match("Strix aluco", &[1.0, 2.0, 3.0]).is_none());
    }

    #[test]
    fn find_match_picks_best() {
        let mut map = HashMap::new();
        map.insert(
            "Tyto alba".to_string(),
            vec![
                ReferenceEntry {
                    individual_id: Uuid::from_bytes([1; 16]),
                    label: "Owl #1".into(),
                    embedding: vec![1.0, 0.0, 0.0],
                },
                ReferenceEntry {
                    individual_id: Uuid::from_bytes([2; 16]),
                    label: "Owl #2".into(),
                    embedding: vec![0.9, 0.1, 0.0], // closer to query
                },
            ],
        );
        let matcher = IndividualMatcher {
            entries: ArcSwap::from_pointee(map),
            db: unreachable_db(),
            threshold: 0.5,
        };
        let result = matcher.find_match("Tyto alba", &[0.9, 0.1, 0.0]).unwrap();
        assert_eq!(result.individual_label, "Owl #2");
    }

    /// Create a Database that panics if used — for tests that only exercise
    /// the in-memory matching logic.
    fn unreachable_db() -> Database {
        // We can't construct a real Database without a file. Use a trick:
        // Clone from a leaked temp db. This is test-only.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("test.db");
            std::mem::forget(dir);
            Database::open(&path).await.unwrap()
        })
    }
}
