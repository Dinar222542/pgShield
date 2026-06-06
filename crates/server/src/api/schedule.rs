use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use chrono::Utc;
use serde::Deserialize;
use uuid::Uuid;

use pgshield_common::*;

use crate::AppState;

pub fn router() -> axum::Router<AppState> {
    axum::Router::new()
        .route("/", axum::routing::get(list_schedules).post(create_schedule))
        .route("/:id", axum::routing::get(get_schedule).put(update_schedule).delete(delete_schedule))
        .route("/:id/toggle", axum::routing::post(toggle_schedule))
}

#[derive(Deserialize)]
pub struct CreateScheduleRequest {
    pub name: String,
    pub storage_id: Uuid,
    pub agent_id: Option<Uuid>,
    pub database: String,
    pub db_host: String,
    pub db_port: u16,
    pub db_user: String,
    pub db_password: String,
    pub cron_expr: String,
    pub retention_days: i32,
    pub compression: Option<String>,
}

#[derive(Deserialize)]
pub struct UpdateScheduleRequest {
    pub name: Option<String>,
    pub storage_id: Option<Uuid>,
    pub agent_id: Option<Uuid>,
    pub database: Option<String>,
    pub db_host: Option<String>,
    pub db_port: Option<u16>,
    pub db_user: Option<String>,
    pub db_password: Option<String>,
    pub cron_expr: Option<String>,
    pub enabled: Option<bool>,
    pub retention_days: Option<i32>,
    pub compression: Option<String>,
}

async fn list_schedules(State(state): State<AppState>) -> Json<Vec<BackupSchedule>> {
    Json(state.db.list_schedules().await)
}

async fn get_schedule(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<BackupSchedule>, StatusCode> {
    state.db.get_schedule(id).await.ok_or(StatusCode::NOT_FOUND).map(Json)
}

async fn create_schedule(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(req): Json<CreateScheduleRequest>,
) -> Result<Json<BackupSchedule>, StatusCode> {
    // Validate cron expression
    if req.cron_expr.parse::<cron::Schedule>().is_err() {
        return Err(StatusCode::BAD_REQUEST);
    }

    let now = Utc::now();
    let schedule = BackupSchedule {
        id: Uuid::new_v4(),
        name: req.name,
        storage_id: req.storage_id,
        agent_id: req.agent_id,
        database: req.database,
        db_host: req.db_host,
        db_port: req.db_port,
        db_user: req.db_user,
        db_password: req.db_password,
        cron_expr: req.cron_expr,
        enabled: true,
        retention_days: req.retention_days,
        compression: req.compression,
        last_run: None,
        last_status: None,
        created_at: now,
        updated_at: now,
    };

    state.db.create_schedule(&schedule).await;
    state.db.log_audit("create", "schedule", &schedule.id.to_string(), &format!("Schedule '{}' created", schedule.name), &claims.sub).await;
    Ok(Json(schedule))
}

async fn update_schedule(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateScheduleRequest>,
) -> Result<Json<BackupSchedule>, StatusCode> {
    let mut schedule = state.db.get_schedule(id).await.ok_or(StatusCode::NOT_FOUND)?;

    if let Some(name) = req.name { schedule.name = name; }
    if let Some(sid) = req.storage_id { schedule.storage_id = sid; }
    if let Some(aid) = req.agent_id { schedule.agent_id = Some(aid); }
    if let Some(db) = req.database { schedule.database = db; }
    if let Some(h) = req.db_host { schedule.db_host = h; }
    if let Some(p) = req.db_port { schedule.db_port = p; }
    if let Some(u) = req.db_user { schedule.db_user = u; }
    if let Some(p) = req.db_password { schedule.db_password = p; }
    if let Some(c) = req.cron_expr {
        if c.parse::<cron::Schedule>().is_err() {
            return Err(StatusCode::BAD_REQUEST);
        }
        schedule.cron_expr = c;
    }
    if let Some(e) = req.enabled { schedule.enabled = e; }
    if let Some(d) = req.retention_days { schedule.retention_days = d; }
    if let Some(c) = req.compression { schedule.compression = Some(c); }

    schedule.updated_at = Utc::now();
    state.db.update_schedule(&schedule).await;
    state.db.log_audit("update", "schedule", &id.to_string(), &format!("Schedule '{}' updated", schedule.name), &claims.sub).await;
    Ok(Json(schedule))
}

async fn delete_schedule(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
) -> StatusCode {
    match state.db.delete_schedule(id).await {
        true => {
            state.db.log_audit("delete", "schedule", &id.to_string(), &format!("Schedule deleted"), &claims.sub).await;
            StatusCode::NO_CONTENT
        }
        false => StatusCode::NOT_FOUND,
    }
}

async fn toggle_schedule(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
) -> Result<Json<BackupSchedule>, StatusCode> {
    let mut schedule = state.db.get_schedule(id).await.ok_or(StatusCode::NOT_FOUND)?;
    schedule.enabled = !schedule.enabled;
    schedule.updated_at = Utc::now();
    state.db.update_schedule(&schedule).await;
    state.db.log_audit("toggle", "schedule", &id.to_string(), &format!("Schedule '{}' toggled to {}", schedule.name, if schedule.enabled { "on" } else { "off" }), &claims.sub).await;
    Ok(Json(schedule))
}
