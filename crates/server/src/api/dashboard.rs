use axum::{extract::State, Json};
use serde::Serialize;

use crate::AppState;

#[derive(Serialize)]
pub struct StatsResponse {
    total_agents: i64,
    online_agents: i64,
    total_backups: i64,
    successful_backups: i64,
    failed_backups: i64,
    total_databases: i64,
    total_storage: i64,
    storage_used_bytes: i64,
    storage_used_human: String,
    last_backup_at: Option<String>,
}

pub fn router() -> axum::Router<AppState> {
    axum::Router::new().route("/stats", axum::routing::get(get_stats))
}

async fn get_stats(State(state): State<AppState>) -> Json<StatsResponse> {
    let s = state.db.get_dashboard_stats().await;
    Json(StatsResponse {
        total_agents: s.total_agents,
        online_agents: s.online_agents,
        total_backups: s.total_backups,
        successful_backups: s.successful_backups,
        failed_backups: s.failed_backups,
        total_databases: s.total_databases,
        total_storage: s.total_storage,
        storage_used_bytes: s.storage_used_bytes,
        storage_used_human: human_size(s.storage_used_bytes),
        last_backup_at: s.last_backup_at.map(|d| d.to_rfc3339()),
    })
}

fn human_size(bytes: i64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit_idx = 0;
    while size > 1024.0 && unit_idx < UNITS.len() - 1 {
        size /= 1024.0;
        unit_idx += 1;
    }
    format!("{:.2} {}", size, UNITS[unit_idx])
}
