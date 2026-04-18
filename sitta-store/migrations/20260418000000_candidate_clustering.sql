-- Candidate clustering for individual identification.
-- Replaces aggressive auto-enrollment with a pool of unmatched
-- embeddings that are periodically clustered. Clusters meeting
-- readiness criteria (min detections + min distinct days) become
-- enrollment suggestions for the user.

-- Unmatched Perch embeddings awaiting clustering.
CREATE TABLE candidate_embeddings (
    detection_id    BLOB(16) NOT NULL PRIMARY KEY REFERENCES detections(id) ON DELETE CASCADE,
    scientific_name TEXT     NOT NULL,
    embedding       BLOB     NOT NULL,  -- raw f32 LE bytes
    cluster_id      INTEGER,            -- NULL = unclustered, set by clustering pass
    created_at      INTEGER  NOT NULL   -- Unix milliseconds
);
CREATE INDEX idx_candidates_species ON candidate_embeddings(scientific_name);
CREATE INDEX idx_candidates_cluster ON candidate_embeddings(cluster_id) WHERE cluster_id IS NOT NULL;

-- Discovered embedding clusters. Each cluster is a potential individual.
CREATE TABLE candidate_clusters (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    scientific_name TEXT    NOT NULL,
    centroid        BLOB   NOT NULL,    -- mean embedding (f32 LE bytes)
    centroid_dim    INTEGER NOT NULL,
    member_count    INTEGER NOT NULL DEFAULT 0,
    distinct_days   INTEGER NOT NULL DEFAULT 0,
    first_seen_at   INTEGER NOT NULL,   -- Unix milliseconds
    last_seen_at    INTEGER NOT NULL,   -- Unix milliseconds
    status          TEXT    NOT NULL DEFAULT 'pending'
                    CHECK(status IN ('pending', 'enrolled', 'dismissed')),
    individual_id   BLOB(16) REFERENCES individuals(id)  -- set when enrolled
);
CREATE INDEX idx_clusters_status ON candidate_clusters(status, scientific_name);
