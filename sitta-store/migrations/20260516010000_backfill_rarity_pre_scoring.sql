-- One-shot historical fix.
--
-- Rarity scoring (`persist::compute_rarity`) went live on 2026-04-19 22:36.
-- Detections recorded before that — roughly 78,000 rows including the
-- station's very first arrivals of every spring-2026 migrant — have no
-- `detection_rarity` row at all. The retention worker treats `rarity =
-- None` as tier 0 ("common"), so those clips age out at the baseline
-- 30 days and lose every size-sweep tiebreaker to noisier post-cutover
-- detections.
--
-- The most visible casualty: American Woodcock. First call 2026-04-18
-- 01:04 (well below the rarity worker's start). Every subsequent call
-- sees `local_count > 0` and is permanently tier 0, so 26,178 detections
-- and a peak confidence of 0.9999 produced zero persisted clips.
--
-- This migration replays the *local* portion of `compute_rarity` over
-- every detection that's eligible by the runtime gate:
--
--   * `confidence >= 0.65`  (the `display_min_confidence` default, which
--     the live config doesn't override; same threshold `species_local_history`
--     uses at runtime)
--   * `label.scientific_name` looks like a Linnaean binomial — i.e. matches
--     the going-forward gate added in `looks_like_species_name`. This
--     excludes Perch's ambient-sound pseudo-labels and AudioSet "Bass_drum"
--     style fragments, which the companion migration
--     20260516000000_drop_non_species_rarity.sql already deleted from the
--     table.
--
-- We compute first_ever / first_season / first_week / first_day using
-- SQLite window functions over the (station, scientific_name) partition,
-- which exactly mirrors `species_local_history`'s scoping. `range_score`
-- is left NULL (the RangeFilter isn't reachable from SQL) and
-- `temporal_score` is set to 0; neither feeds the retention tier, only
-- the composite `score`. The score uses local × 0.40 weight, which is
-- what `compute_rarity` would produce when regional and temporal data
-- are absent — high enough for `first_ever` to land in tier 3 via the
-- existing `tier()` discriminator (which keys on the boolean flags, not
-- the score).
--
-- Rows that already have a `detection_rarity` entry are left untouched;
-- the INSERT skips them via `WHERE has_rarity = 0`.

INSERT INTO detection_rarity (
    detection_id, score, first_ever, first_season, first_week, first_day,
    days_since_last, local_count, range_score, temporal_score
)
WITH qualifying AS (
    -- All real-species detections at or above the display threshold.
    -- We walk the entire stream — not just the missing rows — because a
    -- backfilled row's "prior history" includes detections that already
    -- have rarity rows. `has_rarity` flags which rows to actually insert.
    SELECT
        d.id              AS detection_id,
        d.detected_at,
        d.station_id,
        l.scientific_name,
        EXISTS (SELECT 1 FROM detection_rarity r WHERE r.detection_id = d.id) AS has_rarity
    FROM detections d
    JOIN labels l ON l.id = d.label_id
    WHERE d.confidence >= 0.65
      AND l.scientific_name LIKE '% %'
),
windowed AS (
    SELECT
        detection_id,
        detected_at,
        has_rarity,
        ROW_NUMBER() OVER w AS rn,
        LAG(detected_at) OVER w AS prev_at
    FROM qualifying
    WINDOW w AS (PARTITION BY station_id, scientific_name ORDER BY detected_at)
),
flagged AS (
    SELECT
        detection_id,
        rn,
        has_rarity,
        prev_at,
        detected_at,
        (rn = 1) AS first_ever,
        -- first_day: UTC calendar date differs from previous detection.
        CASE
            WHEN rn = 1 THEN 1
            WHEN date(detected_at/1000, 'unixepoch')
              != date(prev_at/1000,    'unixepoch') THEN 1
            ELSE 0
        END AS first_day,
        -- first_week: year-week. SQLite %W is Monday-first (week-01 is the
        -- first Monday), which differs from chrono's ISO-8601 week only at
        -- year-boundary edge cases. None fall in the backfill window
        -- (everything is April-May 2026).
        CASE
            WHEN rn = 1 THEN 1
            WHEN strftime('%Y-%W', detected_at/1000, 'unixepoch')
              != strftime('%Y-%W', prev_at/1000,    'unixepoch') THEN 1
            ELSE 0
        END AS first_week,
        -- first_season: meteorological season change. The mapping
        --   month → ((month + 9) / 3) % 4   (integer division)
        -- yields 0=Spring, 1=Summer, 2=Autumn, 3=Winter for the Northern
        -- hemisphere. The Southern shift is `(s + 2) % 4`, but since both
        -- sides of the comparison live in the same partition (same
        -- station, therefore same hemisphere), the shift cancels — the
        -- comparison is hemisphere-agnostic without us needing to read
        -- `stations.latitude`.
        CASE
            WHEN rn = 1 THEN 1
            WHEN ((CAST(strftime('%m', detected_at/1000, 'unixepoch') AS INTEGER) + 9) / 3) % 4
              != ((CAST(strftime('%m', prev_at/1000,    'unixepoch') AS INTEGER) + 9) / 3) % 4 THEN 1
            ELSE 0
        END AS first_season,
        CASE
            WHEN rn = 1 THEN NULL
            ELSE CAST((detected_at - prev_at) / 86400000 AS INTEGER)
        END AS days_since_last,
        rn - 1 AS local_count
    FROM windowed
)
SELECT
    detection_id,
    -- Score: local component only, weighted at 0.40 (matches `compute_rarity`
    -- when range_score / temporal_score are absent). Threshold for the
    -- retention worker's "high_score" tier (≥0.6) is unreachable from local
    -- alone, which is correct — these old rows simply don't have the
    -- regional/temporal signal to justify a high-score boost.
    CASE
        WHEN first_ever   = 1 THEN 0.40
        WHEN first_season = 1 THEN 0.32                                       -- 0.8 × 0.40
        WHEN first_week   = 1 THEN
            (0.5 + MIN(COALESCE(days_since_last, 0) / 30.0, 1.0) * 0.3) * 0.40
        WHEN first_day    = 1 THEN
            (0.2 + MIN(COALESCE(days_since_last, 0) / 30.0, 1.0) * 0.2) * 0.40
        ELSE
            MIN(COALESCE(days_since_last, 0) / 30.0, 1.0) * 0.1 * 0.40
    END AS score,
    first_ever, first_season, first_week, first_day,
    days_since_last, local_count,
    NULL AS range_score,
    0    AS temporal_score
FROM flagged
WHERE has_rarity = 0;
