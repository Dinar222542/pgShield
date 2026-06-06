pub mod agents;
pub mod backups;
pub mod dashboard;
pub mod schedule;
pub mod storage;

use axum::{Router, Json, extract::State, extract::Query, http::StatusCode, Extension};
use pgshield_common::{Claims, RestoreRequest, RestoreResult, User};
use uuid::Uuid;
use serde::Deserialize;
use serde_json::json;
use tower_http::cors::CorsLayer;

use crate::system;
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/health", axum::routing::get(health))
        .route("/api/redis", axum::routing::get(redis_info))
        .route("/api/system", axum::routing::get(system_info))
        .route("/api/metrics/history", axum::routing::get(metrics_history))
        .route("/api/metrics/config", axum::routing::get(get_metrics_config).put(set_metrics_config))
        .route("/api/metrics/clear", axum::routing::post(clear_metrics))
        .route("/api/audit", axum::routing::get(get_audit))
        .route("/api/users", axum::routing::get(list_users).post(create_user))
        .route("/api/users/:username", axum::routing::delete(delete_user))
        .nest("/api/dashboard", dashboard::router())
        .nest("/api/agents", agents::router())
        .nest("/api/backups", backups::router())
        .nest("/api/storage", storage::router())
        .nest("/api/schedule", schedule::router())
        .route("/api/restore", axum::routing::post(restore_backup))
        .layer(CorsLayer::permissive())
}

async fn health() -> &'static str {
    "OK"
}

async fn redis_info(State(state): State<AppState>) -> Json<pgshield_common::RedisHealth> {
    Json(state.db.redis_health().await)
}

async fn system_info() -> Json<crate::system::SystemInfo> {
    Json(system::gather())
}

#[derive(Deserialize)]
struct HistoryRange {
    range: Option<String>,
}

async fn metrics_history(
    State(state): State<AppState>,
    Query(q): Query<HistoryRange>,
) -> Json<Vec<pgshield_common::MetricsSnapshot>> {
    let range_secs = match q.range.as_deref() {
        Some("7d") => 7 * 86400,
        Some("30d") => 30 * 86400,
        _ => 86400, // default 24h
    };
    Json(state.db.get_metrics_history(range_secs).await)
}

async fn get_metrics_config(State(state): State<AppState>) -> Json<serde_json::Value> {
    let ttl = state.db.get_metrics_ttl().await;
    Json(serde_json::json!({"ttl_days": ttl}))
}

async fn set_metrics_config(
    State(state): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if let Some(days) = body.get("ttl_days").and_then(|v| v.as_i64()) {
        let clamped = days.clamp(1, 365);
        state.db.set_metrics_ttl(clamped).await;
        state.db.log_audit("update", "metrics_config", "", &format!("TTL set to {} days", clamped), "admin").await;
        Ok(Json(serde_json::json!({"ttl_days": clamped})))
    } else {
        Err(StatusCode::BAD_REQUEST)
    }
}

async fn clear_metrics(State(state): State<AppState>) -> Json<serde_json::Value> {
    state.db.clear_metrics_history().await;
    state.db.log_audit("clear", "metrics", "", "Metrics history cleared", "admin").await;
    Json(serde_json::json!({"ok": true}))
}

#[derive(Deserialize)]
struct AuditQuery {
    limit: Option<usize>,
    offset: Option<usize>,
}

async fn get_audit(
    State(state): State<AppState>,
    Query(q): Query<AuditQuery>,
) -> Json<Vec<pgshield_common::AuditEntry>> {
    let limit = q.limit.unwrap_or(50).min(200);
    let offset = q.offset.unwrap_or(0);
    Json(state.db.get_audit_logs(limit, offset).await)
}

#[derive(Deserialize)]
struct CreateUserRequest {
    username: String,
    password: String,
    role: Option<String>,
}

async fn list_users(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> Result<Json<Vec<serde_json::Value>>, StatusCode> {
    let users = state.db.list_users().await;
    let safe: Vec<_> = users.into_iter().map(|u| json!({
        "username": u.username,
        "role": u.role,
        "created_at": u.created_at,
    })).collect();
    Ok(Json(safe))
}

async fn create_user(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(req): Json<CreateUserRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if req.username.is_empty() || req.password.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }
    if state.db.get_user(&req.username).await.is_some() {
        return Err(StatusCode::CONFLICT);
    }
    let hash = bcrypt::hash(&req.password, 4).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let user = User {
        username: req.username.clone(),
        password_hash: hash,
        role: req.role.unwrap_or_else(|| "admin".into()),
        created_at: chrono::Utc::now().timestamp(),
    };
    state.db.create_user(&user).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    state.db.log_audit("create", "user", &req.username, &format!("User '{}' created with role '{}'", req.username, user.role), &claims.sub).await;
    Ok(Json(json!({"username": req.username, "role": user.role, "created_at": user.created_at})))
}

async fn delete_user(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    axum::extract::Path(username): axum::extract::Path<String>,
) -> StatusCode {
    if username == "admin" {
        return StatusCode::FORBIDDEN;
    }
    if state.db.delete_user(&username).await {
        state.db.log_audit("delete", "user", &username, &format!("User '{}' deleted", username), &claims.sub).await;
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

async fn restore_backup(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(req): Json<RestoreRequest>,
) -> Result<Json<RestoreResult>, StatusCode> {
    let backup = state.db.get_backup(req.backup_id).await.ok_or(StatusCode::NOT_FOUND)?;
    let file_path = backup.file_path.as_ref().ok_or(StatusCode::BAD_REQUEST)?;

    let result = if backup.agent_id != Uuid::nil() {
        if let Some(agent) = state.db.get_agent(backup.agent_id).await {
            state.db.call_agent_restore(
                &agent,
                file_path,
                &req.target_db,
                &req.target_host,
                req.target_port,
                &req.target_user,
                &req.target_password,
            ).await
        } else {
            state.db.run_restore_local(
                file_path,
                &req.target_db,
                &req.target_host,
                req.target_port,
                &req.target_user,
                &req.target_password,
            ).await
        }
    } else {
        state.db.run_restore_local(
            file_path,
            &req.target_db,
            &req.target_host,
            req.target_port,
            &req.target_user,
            &req.target_password,
        ).await
    };

    match result {
        Ok(msg) => {
            state.db.log_audit("restore", "backup", &req.backup_id.to_string(), &format!("Restore to '{}@{}:{}' completed", req.target_db, req.target_host, req.target_port), &claims.sub).await;
            Ok(Json(RestoreResult { success: true, message: msg }))
        }
        Err(e) => {
            state.db.log_audit("restore", "backup", &req.backup_id.to_string(), &format!("Restore failed: {}", e), &claims.sub).await;
            Ok(Json(RestoreResult { success: false, message: e }))
        }
    }
}
