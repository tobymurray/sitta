-- Repair labels that were seeded as `label_type='environment'` because the
-- raw label string was a bare binomial (e.g. "Dryobates villosus") with no
-- underscore-separated common-name suffix and no taxonomy match.
--
-- Before this migration, those rows had `scientific_name=NULL`, so the API
-- returned an empty string, the dashboard rendered `/species/` (broken
-- link), and bucketing in `dashboard_feed_handler` couldn't group them
-- with the same species' detections from other models. The user saw a
-- duplicate, dead-end card alongside the working one.
--
-- Step 1: For environment-type rows whose common_name matches the binomial
-- shape "Genus species" (Capitalised first word, lowercase second word),
-- promote them to species and copy common_name → scientific_name. This
-- alone unblocks the link and re-merges the buckets.
--
-- Step 2: After Step 1 the row has scientific_name == common_name (a
-- degenerate display where the title and italic subtitle are the same).
-- Look for a sister row (any model) that already carries a proper common
-- name for this scientific name and copy it across.

UPDATE labels
SET scientific_name = common_name,
    label_type      = 'species'
WHERE label_type = 'environment'
  AND scientific_name IS NULL
  AND common_name GLOB '[A-Z][a-z]* [a-z][a-z]*';

UPDATE labels
SET common_name = (
    SELECT l2.common_name
    FROM labels l2
    WHERE l2.scientific_name = labels.scientific_name
      AND l2.common_name != l2.scientific_name
      AND l2.id != labels.id
    LIMIT 1
)
WHERE label_type = 'species'
  AND scientific_name IS NOT NULL
  AND scientific_name = common_name
  AND EXISTS (
      SELECT 1 FROM labels l2
      WHERE l2.scientific_name = labels.scientific_name
        AND l2.common_name != l2.scientific_name
        AND l2.id != labels.id
  );
