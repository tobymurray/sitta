-- Rarity scoring for detections.
--
-- Each row captures how unusual a detection was along three axes:
--   local    – novelty at this station (first-ever, first-of-season, etc.)
--   regional – how expected the species is at this location/date (BirdNET meta-model)
--   temporal – how unusual the time-of-day is for this species
--
-- A composite `score` (0.0 = common, 1.0 = extremely rare) summarises the three.

CREATE TABLE detection_rarity (
    detection_id  BLOB(16) NOT NULL PRIMARY KEY REFERENCES detections(id),

    -- Composite rarity score, 0.0 (common) to 1.0 (extremely rare).
    score         REAL     NOT NULL,

    -- Local rarity flags.
    first_ever    BOOLEAN  NOT NULL DEFAULT FALSE,
    first_season  BOOLEAN  NOT NULL DEFAULT FALSE,
    first_week    BOOLEAN  NOT NULL DEFAULT FALSE,
    first_day     BOOLEAN  NOT NULL DEFAULT FALSE,
    days_since_last INTEGER,          -- NULL when first_ever
    local_count   INTEGER  NOT NULL,  -- prior detections of this species at this station

    -- Regional rarity: BirdNET meta-model location score (0.0–1.0).
    -- NULL when range filter is not configured.
    range_score   REAL,

    -- Temporal rarity: how unusual is the detection hour for this species (0.0–1.0).
    temporal_score REAL    NOT NULL DEFAULT 0.0
);

CREATE INDEX idx_rarity_score ON detection_rarity (score DESC);
