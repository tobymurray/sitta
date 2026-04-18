# Runtime Settings Plan

Allow configuration changes via the web UI and API without restarting.

---

## Runtime vs Restart

**Hot (runtime-changeable):**
- `station.name`, `station.latitude`, `station.longitude`
- `inference.birdnet.min_confidence`, `inference.birdnet.top_k`
- `inference.birdnet.meta_threshold`, `inference.birdnet.force_allow`
- `inference.perch.min_confidence`, `inference.perch.top_k`

**Cold (restart-required):**
- `station.id`, `audio.*`, `store.path`, `api.bind`, `taxonomy.ebird_path`
- All `model_path` and `labels_path` fields
- `inference.birdnet.meta_model_path`

## Architecture

- `arc-swap::ArcSwap<RuntimeSettings>` for lock-free reads on the hot path
- `tokio::sync::watch` to notify consumers when settings change
- `toml_edit` to persist changes back to `config.toml` preserving comments
- Classifier rebuild in consumer task (not API handler) via `spawn_blocking`

## API

- `GET /api/v1/settings` — current settings with `_meta.restart_required` list
- `PUT /api/v1/settings` — partial update, rejects restart-required fields

## Steps

1. Add dependencies (`arc-swap`, `toml_edit`) ✓
2. `RuntimeSettings` type + `SettingsUpdate` + validation + disk persistence ✓
3. Wire `ArcSwap` + watch into `ApiState` and consumer tasks ✓
4. Settings API endpoints (GET/PUT) ✓
5. Settings dashboard page with form ✓
6. Consumer rebuild logic on settings change — deferred until needed
   (The watch channel is in place; consumers will subscribe when we add
   hot-reload of classifiers. For now, threshold changes persist to
   config.toml and take effect on next restart.)
