use axum::{
    extract::State,
    http::{Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Json, Response},
};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use pgshield_common::Claims;
use serde::Deserialize;

use crate::AppState;

#[derive(Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

pub async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> impl IntoResponse {
    let user = state.db.authenticate_user(&req.username, &req.password).await;

    let username = if let Some(ref u) = user {
        u.username.clone()
    } else if state.config.auth.enabled
        && req.username == state.config.auth.username
        && bcrypt::verify(&req.password, &state.auth_password_hash).unwrap_or(false)
    {
        req.username.clone()
    } else {
        if state.config.auth.enabled {
            return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({"error":"Invalid credentials"}))).into_response();
        }
        // Auth disabled — accept as admin
        "admin".to_string()
    };

    let now = chrono::Utc::now().timestamp() as usize;
    let claims = Claims {
        sub: username.clone(),
        exp: now + 86400,
    };

    match encode(&Header::default(), &claims, &EncodingKey::from_secret(state.auth_jwt_secret.as_bytes())) {
        Ok(token) => {
            state.db.log_audit("login", "auth", "", "User logged in", &username).await;
            (StatusCode::OK, Json(serde_json::json!({"token": token}))).into_response()
        }
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error":"Token generation failed"}))).into_response(),
    }
}

pub async fn auth_middleware(
    State(state): State<AppState>,
    mut req: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    if !state.config.auth.enabled {
        req.extensions_mut().insert(Claims {
            sub: "admin".into(),
            exp: usize::MAX,
        });
        return Ok(next.run(req).await);
    }

    let path = req.uri().path();
    if path == "/api/auth/login" || path == "/api/health" || !path.starts_with("/api/") {
        return Ok(next.run(req).await);
    }

    let auth_header = req
        .headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or(StatusCode::UNAUTHORIZED)?;

    let token = decode::<Claims>(
        auth_header,
        &DecodingKey::from_secret(state.auth_jwt_secret.as_bytes()),
        &Validation::default(),
    )
    .map_err(|_| StatusCode::UNAUTHORIZED)?;

    req.extensions_mut().insert(token.claims);
    Ok(next.run(req).await)
}
