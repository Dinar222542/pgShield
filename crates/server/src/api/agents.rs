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
        .route("/", axum::routing::get(list_agents).post(register_agent))
        .route("/:id", axum::routing::get(get_agent).delete(delete_agent))
}

async fn list_agents(State(state): State<AppState>) -> Json<Vec<Agent>> {
    Json(state.db.list_agents().await)
}

#[derive(Deserialize)]
pub struct RegisterRequest {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub version: String,
}

async fn register_agent(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(req): Json<RegisterRequest>,
) -> Result<Json<Agent>, StatusCode> {
    let now = Utc::now();
    let agent = Agent {
        id: Uuid::new_v4(),
        name: req.name,
        host: req.host,
        port: req.port,
        version: req.version,
        status: AgentStatus::Online,
        last_seen: Some(now),
        created_at: now,
        updated_at: now,
    };
    state.db.register_agent(&agent).await.map_err(|e| {
        tracing::error!("Failed to register agent: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    state.db.log_audit("create", "agent", &agent.id.to_string(), &format!("Agent '{}' @ {}:{} registered", agent.name, agent.host, agent.port), &claims.sub).await;
    Ok(Json(agent))
}

async fn get_agent(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Agent>, StatusCode> {
    state.db.get_agent(id).await.ok_or(StatusCode::NOT_FOUND).map(Json)
}

async fn delete_agent(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<Uuid>,
) -> StatusCode {
    let name = state.db.get_agent(id).await.map(|a| a.name).unwrap_or_default();
    match state.db.delete_agent(id).await {
        Ok(true) => {
            state.db.log_audit("delete", "agent", &id.to_string(), &format!("Agent '{}' deleted", name), &claims.sub).await;
            StatusCode::NO_CONTENT
        }
        Ok(false) => StatusCode::NOT_FOUND,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}
