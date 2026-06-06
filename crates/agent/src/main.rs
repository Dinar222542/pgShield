mod config;

use std::net::SocketAddr;
use std::sync::Arc;
use std::path::PathBuf;

use axum::{
    extract::State,
    http::StatusCode,
    Json, Router,
};
use chrono::Utc;
use clap::Parser;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tracing::info;
use uuid::Uuid;

use pgshield_common::BackupStatus;

use crate::config::AgentConfig;

#[derive(Clone)]
struct AppState {
    config: Arc<AgentConfig>,
    jobs: Arc<Mutex<Vec<RunningJob>>>,
}

#[derive(Debug, Clone)]
struct RunningJob {
    id: Uuid,
    started_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
struct BackupRequest {
    pub database: String,
    pub db_host: String,
    pub db_port: u16,
    pub db_user: String,
    pub db_password: String,
    pub storage_path: Option<String>,
}

#[derive(Debug, Serialize)]
struct BackupResponse {
    pub success: bool,
    pub message: String,
    pub file_path: Option<String>,
    pub file_size: Option<i64>,
    pub checksum: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RestoreRequest {
    pub backup_path: String,
    pub target_db: String,
    pub target_host: String,
    pub target_port: u16,
    pub target_user: String,
    pub target_password: String,
}

#[derive(Debug, Serialize)]
struct RestoreResponse {
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    pub status: String,
    pub version: String,
    pub hostname: String,
    pub uptime: i64,
}

#[derive(Parser)]
#[command(name = "pgshield-agent", version, about = "pgShield Backup Agent")]
struct Cli {
    #[arg(long, default_value = "config/agent.yaml")]
    config: String,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cli = Cli::parse();
    let config = Arc::new(AgentConfig::load(&cli.config));

    std::fs::create_dir_all(&config.backup_dir).ok();

    let state = AppState {
        config,
        jobs: Arc::new(Mutex::new(Vec::new())),
    };

    let app = Router::new()
        .route("/api/v1/health", axum::routing::get(health))
        .route("/api/v1/backup", axum::routing::post(run_backup))
        .route("/api/v1/restore", axum::routing::post(run_restore))
        .with_state(state);

    let addr: SocketAddr = format!("{}:{}", "0.0.0.0", 9443)
        .parse()
        .expect("Invalid address");
    info!("pgShield Agent starting on {addr}");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Failed to bind");
    axum::serve(listener, app)
        .await
        .expect("Server error");
}

async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        hostname: hostname().await,
        uptime: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64,
    })
}

async fn hostname() -> String {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .and_then(|o| {
            String::from_utf8(o.stdout)
                .ok()
                .map(|s| s.trim().to_string())
        })
        .unwrap_or_else(|| "unknown".into())
}

async fn run_backup(
    State(state): State<AppState>,
    Json(req): Json<BackupRequest>,
) -> Result<Json<BackupResponse>, StatusCode> {
    let job_id = Uuid::new_v4();
    let ts = Utc::now().format("%Y%m%d_%H%M%S");
    let filename = format!("{}_{}.dump", req.database, ts);
    let file_path = PathBuf::from(&state.config.backup_dir).join(&filename);

    info!("Starting backup job {job_id}: {} -> {}", req.database, file_path.display());

    // Build PGPASSWORD env for pg_dump
    let output = std::process::Command::new("pg_dump")
        .env("PGPASSWORD", &req.db_password)
        .args([
            "-h", &req.db_host,
            "-p", &req.db_port.to_string(),
            "-U", &req.db_user,
            "-F", "c",
            "-f", file_path.to_str().unwrap(),
            &req.database,
        ])
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let metadata = std::fs::metadata(&file_path).ok();
            let file_size = metadata.map(|m| m.len() as i64).unwrap_or(0);

            let checksum = sha256_file(&file_path).await;

            info!("Backup job {job_id} completed: {} bytes", file_size);

            Ok(Json(BackupResponse {
                success: true,
                message: format!("Backup of '{}' completed", req.database),
                file_path: Some(file_path.to_str().unwrap().to_string()),
                file_size: Some(file_size),
                checksum,
            }))
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            tracing::error!("Backup job {job_id} failed: {stderr}");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
        Err(e) => {
            tracing::error!("Backup job {job_id} error: {e}");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

async fn run_restore(
    State(_state): State<AppState>,
    Json(req): Json<RestoreRequest>,
) -> Result<Json<RestoreResponse>, StatusCode> {
    let backup_path = PathBuf::from(&req.backup_path);

    if !backup_path.exists() {
        return Ok(Json(RestoreResponse {
            success: false,
            message: format!("Backup file not found: {}", backup_path.display()),
        }));
    }

    info!("Starting restore: {} -> {}@{}:{}/{}",
        backup_path.display(), req.target_db, req.target_host, req.target_port, req.target_db);

    let output = std::process::Command::new("pg_restore")
        .env("PGPASSWORD", &req.target_password)
        .args([
            "-h", &req.target_host,
            "-p", &req.target_port.to_string(),
            "-U", &req.target_user,
            "-d", &req.target_db,
            "-c",
            "--if-exists",
            "-v",
            backup_path.to_str().unwrap(),
        ])
        .output();

    match output {
        Ok(out) if out.status.success() => {
            info!("Restore completed successfully");
            Ok(Json(RestoreResponse {
                success: true,
                message: "Restore completed successfully".into(),
            }))
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            tracing::error!("Restore failed: {stderr}");
            Ok(Json(RestoreResponse {
                success: false,
                message: format!("Restore failed: {}", stderr.lines().last().unwrap_or(&stderr)),
            }))
        }
        Err(e) => {
            tracing::error!("Restore error: {e}");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

async fn sha256_file(path: &PathBuf) -> Option<String> {
    use sha2::Digest;
    tokio::task::spawn_blocking({
        let path = path.clone();
        move || {
            use std::io::Read;
            let mut file = std::fs::File::open(&path).ok()?;
            let mut hasher = sha2::Sha256::new();
            let mut buf = [0u8; 65536];
            loop {
                let n = file.read(&mut buf).ok()?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
            }
            Some(format!("{:x}", hasher.finalize()))
        }
    })
    .await
    .ok()
    .flatten()
}
