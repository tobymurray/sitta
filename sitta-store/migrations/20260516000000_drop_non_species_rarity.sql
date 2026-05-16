-- One-shot historical fix.
--
-- The rarity scorer was joining history by exact scientific_name match.
-- Perch ships ~200 "pseudo-species" labels for ambient sounds — bare
-- common-name strings ("Animal", "Vehicle", "Music", "Hands", "Thunder",
-- ...) seeded with `scientific_name = NULL`, plus compound AudioSet labels
-- like "Bass_drum" / "Acoustic_guitar" that the seeder split into a
-- single-token scientific_name ("Bass", "Acoustic", ...).
--
-- At runtime, every detection of such a label looked "never seen before"
-- to `species_local_history`, so it was tagged `first_ever = true`. The
-- retention worker then applied the `first_ever_multiplier` (default 999×)
-- and pinned the clip on disk forever. Investigation snapshot:
--   3,799 first_ever clips → 1,548 MB → 60 % of the 2 GB clip budget.
-- That budget pressure was squeezing genuine tier-1 rare-bird clips
-- (American Woodcock, Northern Cardinal, Barred Owl) out of the size sweep.
--
-- The going-forward fix lives in `sitta-bin::persist::compute_rarity`,
-- which now short-circuits to `None` for non-binomial scientific names.
-- This migration removes the rows that were already written before that
-- gate existed. The retention worker re-tiers them to `tier(None) = 0` on
-- the next sweep, so the size sweep can finally evict them and reclaim
-- the budget for actual birds.
--
-- Two predicates cover the bad rows:
--   1. `scientific_name IS NULL` — the 120 `label_type='environment'`
--      entries (Animal, Vehicle, Music, …).
--   2. `scientific_name NOT LIKE '% %'` — the ~88 `label_type='species'`
--      rows that the AudioSet-style "Bass_drum" labels turned into. A real
--      Linnaean binomial always has a space between genus and species.

DELETE FROM detection_rarity
WHERE detection_id IN (
    SELECT d.id
    FROM detections d
    JOIN labels l ON l.id = d.label_id
    WHERE l.scientific_name IS NULL
       OR l.scientific_name NOT LIKE '% %'
);
