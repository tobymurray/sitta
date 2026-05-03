//! Dashboard page handlers — render the embedded HTML shell + page content.
//!
//! These are presentation-only; they don't touch the database or settings
//! beyond reading the timezone for `dashboard::page`. Server-side data is
//! fetched by client-side JS via the JSON endpoints.

use axum::extract::{Path, State};

use crate::dashboard;
use crate::server::ApiState;

pub(super) async fn dashboard_page(
    State(state): State<ApiState>,
) -> axum::response::Html<String> {
    let s = state.core.settings.load();
    let content = dashboard::dashboard_content(&s.station_name);
    dashboard::page("Dashboard", "dashboard", &content, &s.timezone)
}

pub(super) async fn species_page(
    State(state): State<ApiState>,
) -> axum::response::Html<String> {
    let s = state.core.settings.load();
    let content = dashboard::species_content();
    dashboard::page("Species", "species", &content, &s.timezone)
}

pub(super) async fn detection_detail_page(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> axum::response::Html<String> {
    let s = state.core.settings.load();
    let content = dashboard::detection_detail_content(&id);
    dashboard::page("Detection", "dashboard", &content, &s.timezone)
}

pub(super) async fn species_detail_page(
    State(state): State<ApiState>,
    Path(name): Path<String>,
) -> axum::response::Html<String> {
    let s = state.core.settings.load();
    let content = dashboard::species_detail_content(&name);
    dashboard::page(&format!("{name} — Species"), "species", &content, &s.timezone)
}

pub(super) async fn rare_page(State(state): State<ApiState>) -> axum::response::Html<String> {
    let s = state.core.settings.load();
    let content = dashboard::rare_content();
    dashboard::page("Rare moments", "rare", &content, &s.timezone)
}

pub(super) async fn status_page(
    State(state): State<ApiState>,
) -> axum::response::Html<String> {
    let s = state.core.settings.load();
    let content = dashboard::status_content(&s.station_name);
    dashboard::page("Status", "status", &content, &s.timezone)
}

pub(super) async fn diagnostics_page(
    State(state): State<ApiState>,
) -> axum::response::Html<String> {
    let s = state.core.settings.load();
    let content = dashboard::diagnostics_content();
    dashboard::page("Audio Health", "diagnostics", &content, &s.timezone)
}

pub(super) async fn individuals_page(
    State(state): State<ApiState>,
) -> axum::response::Html<String> {
    let s = state.core.settings.load();
    let content = dashboard::individuals_content();
    dashboard::page("Individuals", "individuals", &content, &s.timezone)
}

pub(super) async fn settings_page(
    State(state): State<ApiState>,
) -> axum::response::Html<String> {
    let s = state.core.settings.load();
    let content = dashboard::settings_content(&s, &state.core.initial_config);
    dashboard::page("Settings", "settings", &content, &s.timezone)
}
