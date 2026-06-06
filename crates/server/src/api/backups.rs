use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Extension, Json,
};
use chrono::Utc;
use serde::Deserialize;
use uuid::Uuid;

use pgshield_common::*;

use crate::AppState;

#[derive(Deserialize)]
pub struct ListParams {
    agent_id: Option<Uuid>,
    storage_id: Option<Uuid>,
}

pub fn router() -> axum::Router<AppState> {
    axum::Router::new()
        .route("/", axum::routing::get(list_backups).post(create_backup))
        .route("/:id", axum::routing::get(get_backup).delete(delete_backup))
}

async fn list_backups(
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> Json<Vec<Backup>> {
    Json(state.db.list_backups(params.agent_id, params.storage_id).await)
}

async fn get_backup(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Backup>, StatusCode> {
    state.db.get_backup(id).await.ok_or(StatusCode::NOT_FOUND).map(Json)
}

#[derive(Deserialize)]
pub struct CreateBackupRequest {
    pub agent_id: Uuid,
    pub storage_id: Option<Uuid>,
    pub database_name: String,
    pub backup_type: String,
    pub retention_days: Option<i32>,
}

async fn create_backup(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(req): Json<CreateBackupRequest>,
) -> Result<Json<Backup>, StatusCode> {
    let now = Utc::now();
    let retention_until = req.retention_days.map(|days| now + chrono::Duration::days(days as i64));
    let backup = Backup {
        id: Uuid::new_v4(),
        agent_id: req.agent_id,
        storage_id: req.storage_id,
        database_name: req.database_name,
        backup_type: match req.backup_type.as_str() {
            "full" => BackupType::Full,
            "incremental" => BackupType::Incremental,
            "wal" => BackupType::WAL,
            _ => return Err(StatusCode::BAD_REQUEST),
        },
        status: BackupStatus::Running,
        file_path: None,
        file_size: None,
        checksum: None,
        error_message: None,
        retention_until,
        created_at: now,
        completed_at: None,
    };
    state.db.create_backup(&backup).await.map_err(|e| {
        tracing::error!("Failed to create backup: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    state.db.log_audit("create", "backup", &backup.id.to_string(), &format!("Backup '{}' ({}) created", backup.database_name, backup.backup_type), &claims.sub).await;
    Ok(Json(backup))
}

async fn delete_backup(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
) -> StatusCode {
    let db_name = state.db.get_backup(id).await.map(|b| b.database_name).unwrap_or_default();
    match state.db.delete_backup(id).await {
        Ok(true) => {
            state.db.log_audit("delete", "backup", &id.to_string(), &format!("Backup '{}' deleted", db_name), &claims.sub).await;
            StatusCode::NO_CONTENT
        }
        Ok(false) => StatusCode::NOT_FOUND,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}
