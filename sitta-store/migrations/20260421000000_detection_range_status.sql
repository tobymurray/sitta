-- Track how each detection relates to the geographic range filter.
-- NULL = pre-migration or no range filter configured.
-- 'allowed' = species in meta-model and passed location/date threshold.
-- 'force_allowed' = species passed via force_allow bypass.
-- 'not_in_meta_model' = species not in BirdNET's label space (Perch-only).
ALTER TABLE detections ADD COLUMN range_status TEXT;
