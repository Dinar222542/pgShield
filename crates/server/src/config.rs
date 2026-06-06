use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub storage: StorageConfig,
    pub auth: AuthConfig,
    pub metrics: MetricsConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseConfig {
    pub redis_url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StorageConfig {
    pub backup_dir: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthConfig {
    pub enabled: bool,
    pub username: String,
    pub password: String,
    pub jwt_secret: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MetricsConfig {
    pub ttl_days: i64,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            server: ServerConfig {
                host: "0.0.0.0".into(),
                port: 8080,
            },
            database: DatabaseConfig {
                redis_url: "redis://127.0.0.1:6379".into(),
            },
            storage: StorageConfig {
                backup_dir: "data/backups".into(),
            },
            auth: AuthConfig {
                enabled: false,
                username: "admin".into(),
                password: "admin".into(),
                jwt_secret: String::new(),
            },
            metrics: MetricsConfig { ttl_days: 30 },
        }
    }
}

impl Config {
    pub fn load(path: &str) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|contents| serde_yaml::from_str(&contents).ok())
            .unwrap_or_default()
    }
}
