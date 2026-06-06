mod api;
mod auth;
mod config;
mod db;
mod scheduler;
mod system;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::{Router, middleware};
use clap::Parser;
use chrono::Utc;
use tower_http::services::ServeDir;
use tracing::info;
use tracing_subscriber::EnvFilter;

use crate::config::Config;
use crate::db::Database;

#[derive(Clone)]
pub struct AppState {
    pub db: Database,
    pub config: Arc<Config>,
    pub auth_jwt_secret: String,
    pub auth_password_hash: String,
}

#[derive(Parser)]
#[command(name = "pgshield-server", version, about = "pgShield Management Server")]
struct Cli {
    #[arg(long, default_value = "config/default.yaml")]
    config: String,

    #[arg(long, default_value = "data")]
    data_dir: String,

    #[arg(long)]
    seed: bool,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let cli = Cli::parse();
    let config = Arc::new(Config::load(&cli.config));

    std::fs::create_dir_all(&cli.data_dir).expect("Failed to create data directory");
    std::fs::create_dir_all(&config.storage.backup_dir)
        .expect("Failed to create backup directory");

    let db = Database::new(&config.database.redis_url, &config.storage.backup_dir)
        .await
        .expect("Failed to connect to Redis");

    if cli.seed {
        db.seed().await;
        db.seed_user("admin", &config.auth.password).await;
    }

    // Background metrics collector
    let db_clone = db.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(15));
        loop {
            interval.tick().await;
            let sys = tokio::task::spawn_blocking(system::gather).await.unwrap_or_default();
            let redis = db_clone.redis_health().await;
            let stats = db_clone.get_dashboard_stats().await;
            let snapshot = pgshield_common::MetricsSnapshot {
                ts: Utc::now().timestamp(),
                cpu_usage: sys.cpu_usage,
                memory_used: sys.memory_used,
                memory_total: sys.memory_total,
                memory_percent: sys.memory_percent,
                disk_used: sys.disk_used,
                disk_total: sys.disk_total,
                disk_percent: sys.disk_percent,
                redis_ping_ms: redis.ping_ms,
                redis_memory: redis.used_memory,
                redis_clients: redis.connected_clients,
                redis_ops_per_sec: redis.ops_per_sec,
                redis_hit_rate: redis.hit_rate,
                total_backups: stats.total_backups,
                total_agents: stats.total_agents,
                total_storage: stats.total_storage,
            };
            db_clone.push_metrics(&snapshot).await;
        }
    });

    // Background scheduler
    let scheduler = scheduler::Scheduler::new(db.clone());
    tokio::spawn(async move {
        scheduler.start().await;
    });

    let auth_jwt_secret = if config.auth.jwt_secret.is_empty() {
        use rand::Rng;
        let mut rng = rand::thread_rng();
        (0..32).map(|_| rng.gen::<u8>()).collect()
    } else {
        config.auth.jwt_secret.clone().into_bytes()
    };
    let auth_password_hash = bcrypt::hash(&config.auth.password, 4).unwrap_or_else(|_| "admin".into());

    let state = AppState {
        db,
        config: config.clone(),
        auth_jwt_secret: String::from_utf8(auth_jwt_secret).unwrap_or_else(|_| "secret".into()),
        auth_password_hash,
    };

    let app = Router::new()
        .route("/api/auth/login", axum::routing::post(auth::login))
        .merge(api::router())
        .fallback_service(ServeDir::new("static").append_index_html_on_directories(true))
        .layer(middleware::from_fn_with_state(state.clone(), auth::auth_middleware))
        .with_state(state);

    let addr = SocketAddr::new(
        config.server.host.parse().expect("Invalid host"),
        config.server.port,
    );
    info!("pgShield server starting on {addr}");
    info!("Dashboard: http://{addr}");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Failed to bind address");
    axum::serve(listener, app)
        .await
        .expect("Server error");
}
