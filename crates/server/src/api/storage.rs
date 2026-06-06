use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use pgshield_common::*;

use crate::AppState;

pub fn router() -> axum::Router<AppState> {
    axum::Router::new()
        .route("/", axum::routing::get(list_storage).post(create_storage))
        .route("/:id", axum::routing::get(get_storage).put(update_storage).delete(delete_storage))
        .route("/:id/test", axum::routing::post(test_storage))
        .route("/:id/retention", axum::routing::get(get_retention).put(update_retention))
}

async fn list_storage(State(state): State<AppState>) -> Json<Vec<StorageBackend>> {
    Json(state.db.list_storage().await)
}

async fn get_storage(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<StorageBackend>, StatusCode> {
    state.db.get_storage(id).await.ok_or(StatusCode::NOT_FOUND).map(Json)
}

#[derive(Debug, Deserialize)]
pub struct CreateStorageRequest {
    pub name: String,
    pub storage_type: String,
    pub config: serde_json::Value,
    pub retention_days: Option<i32>,
    pub compression: Option<String>,
    pub dedup_enabled: Option<bool>,
    pub description: Option<String>,
}

async fn create_storage(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(req): Json<CreateStorageRequest>,
) -> Result<Json<StorageBackend>, StatusCode> {
    let s_type = StorageType::from_str(&req.storage_type).ok_or(StatusCode::BAD_REQUEST)?;
    let config = build_config(&s_type, &req.config).map_err(|_| StatusCode::BAD_REQUEST)?;
    let compression = req.compression
        .and_then(|c| CompressionType::from_str(&c))
        .unwrap_or(CompressionType::Zstd);
    let now = Utc::now();

    let storage = StorageBackend {
        id: Uuid::new_v4(),
        name: req.name,
        storage_type: s_type,
        config,
        retention_days: req.retention_days.unwrap_or(30),
        compression,
        dedup_enabled: req.dedup_enabled.unwrap_or(false),
        status: StorageStatus::Unknown,
        total_space: None,
        used_space: None,
        description: req.description.unwrap_or_default(),
        created_at: now,
        updated_at: now,
    };

    state.db.create_storage(&storage).await.map_err(|e| {
        tracing::error!("Failed to create storage: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    state.db.log_audit("create", "storage", &storage.id.to_string(), &format!("Storage '{}' ({}) created", storage.name, storage.storage_type), &claims.sub).await;
    Ok(Json(storage))
}

#[derive(Debug, Deserialize)]
pub struct UpdateStorageRequest {
    pub name: Option<String>,
    pub storage_type: Option<String>,
    pub config: Option<serde_json::Value>,
    pub retention_days: Option<i32>,
    pub compression: Option<String>,
    pub dedup_enabled: Option<bool>,
    pub status: Option<String>,
    pub total_space: Option<i64>,
    pub used_space: Option<i64>,
    pub description: Option<String>,
}

async fn update_storage(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateStorageRequest>,
) -> Result<Json<StorageBackend>, StatusCode> {
    let mut storage = state.db.get_storage(id).await.ok_or(StatusCode::NOT_FOUND)?;

    if let Some(name) = req.name { storage.name = name; }
    if let Some(s_type) = req.storage_type {
        storage.storage_type = StorageType::from_str(&s_type).ok_or(StatusCode::BAD_REQUEST)?;
    }
    if let Some(config) = req.config {
        storage.config = build_config(&storage.storage_type, &config).map_err(|_| StatusCode::BAD_REQUEST)?;
    }
    if let Some(days) = req.retention_days { storage.retention_days = days; }
    if let Some(comp) = req.compression {
        storage.compression = CompressionType::from_str(&comp).ok_or(StatusCode::BAD_REQUEST)?;
    }
    if let Some(dedup) = req.dedup_enabled { storage.dedup_enabled = dedup; }
    if let Some(status) = req.status {
        storage.status = match status.as_str() {
            "connected" => StorageStatus::Connected,
            "disconnected" => StorageStatus::Disconnected,
            "error" => StorageStatus::Error,
            _ => StorageStatus::Unknown,
        };
    }
    if let Some(space) = req.total_space { storage.total_space = Some(space); }
    if let Some(space) = req.used_space { storage.used_space = Some(space); }
    if let Some(desc) = req.description { storage.description = desc; }
    storage.updated_at = Utc::now();

    state.db.update_storage(&storage).await.map_err(|e| {
        tracing::error!("Failed to update storage: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    state.db.log_audit("update", "storage", &storage.id.to_string(), &format!("Storage '{}' updated", storage.name), &claims.sub).await;
    Ok(Json(storage))
}

#[derive(Serialize)]
pub struct TestResult {
    pub success: bool,
    pub message: String,
}

async fn test_storage(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Json<TestResult> {
    match state.db.test_storage_connection(id).await {
        Ok(msg) => Json(TestResult { success: true, message: msg }),
        Err(msg) => Json(TestResult { success: false, message: msg }),
    }
}

async fn delete_storage(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
) -> StatusCode {
    let name = state.db.get_storage(id).await.map(|s| s.name).unwrap_or_default();
    match state.db.delete_storage(id).await {
        Ok(true) => {
            state.db.log_audit("delete", "storage", &id.to_string(), &format!("Storage '{}' deleted", name), &claims.sub).await;
            StatusCode::NO_CONTENT
        }
        Ok(false) => StatusCode::NOT_FOUND,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

// ── Retention ──

#[derive(Debug, Deserialize)]
pub struct RetentionRequest {
    pub keep_last: Option<i32>,
    pub keep_hourly: Option<i32>,
    pub keep_daily: Option<i32>,
    pub keep_weekly: Option<i32>,
    pub keep_monthly: Option<i32>,
}

async fn get_retention(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<RetentionPolicy>, StatusCode> {
    state.db.get_retention_policy(id).await.ok_or(StatusCode::NOT_FOUND).map(Json)
}

async fn update_retention(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<RetentionRequest>,
) -> Result<Json<RetentionPolicy>, StatusCode> {
    let existing = state.db.get_retention_policy(id).await.unwrap_or_default();
    let policy = RetentionPolicy {
        keep_last: req.keep_last.unwrap_or(existing.keep_last),
        keep_hourly: req.keep_hourly.unwrap_or(existing.keep_hourly),
        keep_daily: req.keep_daily.unwrap_or(existing.keep_daily),
        keep_weekly: req.keep_weekly.unwrap_or(existing.keep_weekly),
        keep_monthly: req.keep_monthly.unwrap_or(existing.keep_monthly),
    };
    state.db.upsert_retention_policy(id, &policy).await.map_err(|e| {
        tracing::error!("Failed to update retention policy: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(Json(policy))
}

// ── Helpers ──

fn build_config(s_type: &StorageType, config: &serde_json::Value) -> Result<StorageConfig, String> {
    match s_type {
        StorageType::Local => {
            serde_json::from_value::<LocalConfig>(config.clone())
                .map(StorageConfig::Local)
                .map_err(|e| format!("Invalid local config: {e}"))
        }
        StorageType::NFS => {
            serde_json::from_value::<NFSConfig>(config.clone())
                .map(StorageConfig::NFS)
                .map_err(|e| format!("Invalid NFS config: {e}"))
        }
        _ => Err("Only Local and NFS storage types are supported".into()),
    }
}
