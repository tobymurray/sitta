//! Greedy single-pass clustering of candidate embeddings.
//!
//! Groups unmatched Perch embeddings into clusters that likely represent
//! the same individual. Each cluster tracks a running centroid (mean
//! embedding), member count, and the number of distinct calendar days
//! spanned by its detections.

use std::collections::HashSet;

use crate::db::Database;
use crate::matcher::cosine_similarity;
use crate::StoreError;

/// Configuration for the clustering pass.
pub struct ClusterConfig {
    /// Minimum cosine similarity to merge a candidate into an existing cluster.
    pub merge_threshold: f32,
    /// Station timezone for computing distinct calendar days.
    pub timezone: String,
    /// Maximum age in days for unclustered candidates (older ones are pruned).
    pub retention_days: u32,
}

/// Run one clustering pass for a single species.
///
/// 1. Load all pending clusters (existing centroids).
/// 2. Load unclustered candidates.
/// 3. For each candidate, find the best matching cluster centroid.
///    - If similarity >= merge_threshold: assign to that cluster.
///    - Otherwise: create a new cluster.
/// 4. Persist assignments and updated cluster stats.
pub async fn cluster_species(
    db: &Database,
    scientific_name: &str,
    config: &ClusterConfig,
) -> Result<ClusterStats, StoreError> {
    let candidates = db.unclustered_candidates(scientific_name).await?;
    if candidates.is_empty() {
        return Ok(ClusterStats::default());
    }

    // Load existing pending clusters as mutable working state.
    let existing = db.pending_clusters(scientific_name).await?;
    let mut clusters: Vec<WorkingCluster> = existing
        .into_iter()
        .map(|c| {
            let emb: Vec<f32> = bytemuck::cast_slice::<u8, f32>(&c.centroid).to_vec();
            WorkingCluster {
                db_id: Some(c.id),
                centroid: emb,
                first_seen: c.first_seen_at,
                last_seen: c.last_seen_at,
                existing_member_count: c.member_count,
                new_members: Vec::new(),
            }
        })
        .collect();

    let mut assigned = 0u64;
    let mut new_clusters = 0u64;

    for candidate in &candidates {
        if candidate.embedding.len() % 4 != 0 {
            tracing::warn!(
                detection = ?candidate.detection_id,
                len = candidate.embedding.len(),
                "Invalid candidate embedding size, skipping"
            );
            continue;
        }
        let emb: &[f32] = bytemuck::cast_slice(&candidate.embedding);

        // Find best matching cluster.
        let mut best_idx: Option<usize> = None;
        let mut best_sim: f32 = 0.0;
        for (i, cluster) in clusters.iter().enumerate() {
            let sim = cosine_similarity(emb, &cluster.centroid);
            if sim >= config.merge_threshold && sim > best_sim {
                best_sim = sim;
                best_idx = Some(i);
            }
        }

        if let Some(idx) = best_idx {
            // Merge into existing cluster: update centroid as running mean.
            let cluster = &mut clusters[idx];
            let n = cluster.total_members() as f32;
            let new_n = n + 1.0;
            for (c, e) in cluster.centroid.iter_mut().zip(emb.iter()) {
                *c = (*c * n + *e) / new_n;
            }
            cluster.new_members.push((
                candidate.detection_id.clone(),
                candidate.created_at,
            ));
            if candidate.created_at < cluster.first_seen {
                cluster.first_seen = candidate.created_at;
            }
            if candidate.created_at > cluster.last_seen {
                cluster.last_seen = candidate.created_at;
            }
            assigned += 1;
        } else {
            // Start a new cluster.
            clusters.push(WorkingCluster {
                db_id: None,
                centroid: emb.to_vec(),
                first_seen: candidate.created_at,
                last_seen: candidate.created_at,
                new_members: vec![(candidate.detection_id.clone(), candidate.created_at)],
                existing_member_count: 0,
            });
            new_clusters += 1;
        }
    }

    // Persist results.
    for cluster in &clusters {
        let centroid_bytes: &[u8] = bytemuck::cast_slice(&cluster.centroid);
        let dim = cluster.centroid.len() as i64;
        let total = cluster.total_members();

        // Compute distinct days from all member timestamps.
        // For existing clusters, we need to include timestamps of previously assigned members.
        let distinct_days = if let Some(cid) = cluster.db_id {
            // Load all member timestamps for this cluster (existing + new).
            compute_distinct_days_from_db(db, cid, &cluster.new_members, &config.timezone).await?
        } else {
            compute_distinct_days(&cluster.new_members, &config.timezone)
        };

        if let Some(cid) = cluster.db_id {
            // Update existing cluster.
            if !cluster.new_members.is_empty() {
                db.update_cluster(
                    cid,
                    centroid_bytes,
                    total,
                    distinct_days,
                    cluster.first_seen,
                    cluster.last_seen,
                )
                .await?;
            }
            // Assign new members.
            for (det_id, _) in &cluster.new_members {
                db.assign_candidate_to_cluster(det_id, cid).await?;
            }
        } else if !cluster.new_members.is_empty() {
            // Create new cluster.
            let cid = db
                .insert_cluster(
                    scientific_name,
                    centroid_bytes,
                    dim,
                    total,
                    distinct_days,
                    cluster.first_seen,
                    cluster.last_seen,
                )
                .await?;
            // Assign members.
            for (det_id, _) in &cluster.new_members {
                db.assign_candidate_to_cluster(det_id, cid).await?;
            }
        }
    }

    Ok(ClusterStats {
        candidates_processed: candidates.len() as u64,
        assigned_to_existing: assigned,
        new_clusters_created: new_clusters,
    })
}

struct WorkingCluster {
    db_id: Option<i64>,
    centroid: Vec<f32>,
    first_seen: i64,
    last_seen: i64,
    existing_member_count: i64,
    /// Newly assigned members: (detection_id bytes, created_at ms).
    new_members: Vec<(Vec<u8>, i64)>,
}

impl WorkingCluster {
    fn total_members(&self) -> i64 {
        self.existing_member_count + self.new_members.len() as i64
    }
}

/// Compute distinct calendar days from a set of timestamps.
fn compute_distinct_days(members: &[(Vec<u8>, i64)], timezone: &str) -> i64 {
    use chrono::{TimeZone, Datelike};
    let tz: chrono_tz::Tz = timezone.parse().unwrap_or(chrono_tz::UTC);
    let days: HashSet<(i32, u32, u32)> = members
        .iter()
        .filter_map(|(_, ts)| {
            let dt = tz.timestamp_millis_opt(*ts).single()?;
            Some((dt.year(), dt.month(), dt.day()))
        })
        .collect();
    days.len() as i64
}

/// Compute distinct days including existing cluster members from DB.
async fn compute_distinct_days_from_db(
    db: &Database,
    cluster_id: i64,
    new_members: &[(Vec<u8>, i64)],
    timezone: &str,
) -> Result<i64, StoreError> {
    use chrono::{TimeZone, Datelike};
    let tz: chrono_tz::Tz = timezone.parse().unwrap_or(chrono_tz::UTC);

    // Get timestamps of existing members from DB.
    let existing = db.cluster_member_timestamps(cluster_id).await?;

    let days: HashSet<(i32, u32, u32)> = existing
        .iter()
        .chain(new_members.iter().map(|(_, ts)| ts))
        .filter_map(|ts| {
            let dt = tz.timestamp_millis_opt(*ts).single()?;
            Some((dt.year(), dt.month(), dt.day()))
        })
        .collect();
    Ok(days.len() as i64)
}

/// Run a full clustering pass across all species with unclustered candidates.
/// Also prunes old unclustered candidates based on retention_days.
pub async fn run_clustering_pass(
    db: &Database,
    config: &ClusterConfig,
) -> Result<FullPassStats, StoreError> {
    let mut total = FullPassStats::default();

    // Prune old candidates.
    if config.retention_days > 0 {
        let cutoff = chrono::Utc::now().timestamp_millis()
            - i64::from(config.retention_days) * 86_400_000;
        total.pruned = db.prune_old_candidates(cutoff).await?;
    }

    // Cluster each species.
    let species_list = db.species_with_unclustered().await?;
    for species in &species_list {
        let stats = cluster_species(db, species, config).await?;
        total.candidates_processed += stats.candidates_processed;
        total.assigned_to_existing += stats.assigned_to_existing;
        total.new_clusters_created += stats.new_clusters_created;
    }

    Ok(total)
}

/// Stats from a single-species clustering pass.
#[derive(Debug, Default)]
pub struct ClusterStats {
    pub candidates_processed: u64,
    pub assigned_to_existing: u64,
    pub new_clusters_created: u64,
}

/// Stats from a full clustering pass across all species.
#[derive(Debug, Default)]
pub struct FullPassStats {
    pub candidates_processed: u64,
    pub assigned_to_existing: u64,
    pub new_clusters_created: u64,
    pub pruned: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinct_days_basic() {
        // Two timestamps on the same day, one on a different day (UTC).
        let members = vec![
            (vec![], 1713398400000), // 2024-04-18 00:00:00 UTC
            (vec![], 1713400200000), // 2024-04-18 00:30:00 UTC
            (vec![], 1713484800000), // 2024-04-19 00:00:00 UTC
        ];
        assert_eq!(compute_distinct_days(&members, "UTC"), 2);
    }

    #[test]
    fn distinct_days_empty() {
        assert_eq!(compute_distinct_days(&[], "UTC"), 0);
    }

    #[test]
    fn distinct_days_timezone_boundary() {
        // Two timestamps that are on different UTC days but same local day (UTC-5).
        // 2024-04-18 23:00 UTC = 2024-04-18 18:00 EST
        // 2024-04-19 03:00 UTC = 2024-04-18 22:00 EST
        let members = vec![
            (vec![], 1713481200000), // 2024-04-18 23:00 UTC
            (vec![], 1713495600000), // 2024-04-19 03:00 UTC
        ];
        assert_eq!(compute_distinct_days(&members, "America/New_York"), 1);
        assert_eq!(compute_distinct_days(&members, "UTC"), 2);
    }
}
