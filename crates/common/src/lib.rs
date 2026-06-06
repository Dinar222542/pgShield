use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ── BackupSchedule ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupSchedule {
    pub id: Uuid,
    pub name: String,
    pub storage_id: Uuid,
    pub agent_id: Option<Uuid>,
    pub database: String,
    pub db_host: String,
    pub db_port: u16,
    pub db_user: String,
    pub db_password: String,
    pub cron_expr: String,
    pub enabled: bool,
    pub retention_days: i32,
    pub compression: Option<String>,
    pub last_run: Option<i64>,
    pub last_status: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ── Restore ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestoreRequest {
    pub backup_id: Uuid,
    #[serde(default)]
    pub storage_id: Option<Uuid>,
    pub target_db: String,
    pub target_host: String,
    pub target_port: u16,
    pub target_user: String,
    pub target_password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestoreResult {
    pub success: bool,
    pub message: String,
}

// ── Agent ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    pub id: Uuid,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub version: String,
    pub status: AgentStatus,
    pub last_seen: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentStatus {
    Online,
    Offline,
    Unknown,
}

impl std::fmt::Display for AgentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentStatus::Online => write!(f, "online"),
            AgentStatus::Offline => write!(f, "offline"),
            AgentStatus::Unknown => write!(f, "unknown"),
        }
    }
}

// ── Backup ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Backup {
    pub id: Uuid,
    pub agent_id: Uuid,
    pub storage_id: Option<Uuid>,
    pub database_name: String,
    pub backup_type: BackupType,
    pub status: BackupStatus,
    pub file_path: Option<String>,
    pub file_size: Option<i64>,
    pub checksum: Option<String>,
    pub error_message: Option<String>,
    pub retention_until: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BackupType {
    Full,
    Incremental,
    WAL,
}

impl std::fmt::Display for BackupType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BackupType::Full => write!(f, "full"),
            BackupType::Incremental => write!(f, "incremental"),
            BackupType::WAL => write!(f, "wal"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BackupStatus {
    Running,
    Completed,
    Failed,
}

impl std::fmt::Display for BackupStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BackupStatus::Running => write!(f, "running"),
            BackupStatus::Completed => write!(f, "completed"),
            BackupStatus::Failed => write!(f, "failed"),
        }
    }
}

// ── Storage Backend ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageBackend {
    pub id: Uuid,
    pub name: String,
    pub storage_type: StorageType,
    pub config: StorageConfig,
    pub retention_days: i32,
    pub compression: CompressionType,
    pub dedup_enabled: bool,
    pub status: StorageStatus,
    pub total_space: Option<i64>,
    pub used_space: Option<i64>,
    pub description: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StorageType {
    Local,
    NFS,
    S3,
    SFTP,
    FC,
    ZFS,
}

impl std::fmt::Display for StorageType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StorageType::Local => write!(f, "local"),
            StorageType::NFS => write!(f, "nfs"),
            StorageType::S3 => write!(f, "s3"),
            StorageType::SFTP => write!(f, "sftp"),
            StorageType::FC => write!(f, "fc"),
            StorageType::ZFS => write!(f, "zfs"),
        }
    }
}

impl StorageType {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "local" => Some(StorageType::Local),
            "nfs" => Some(StorageType::NFS),
            "s3" => Some(StorageType::S3),
            "sftp" => Some(StorageType::SFTP),
            "fc" => Some(StorageType::FC),
            "zfs" => Some(StorageType::ZFS),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StorageConfig {
    Local(LocalConfig),
    NFS(NFSConfig),
    S3(S3Config),
    SFTP(SFTPConfig),
    FC(FCConfig),
    ZFS(ZFSConfig),
}

impl StorageConfig {
    pub fn serialize_inner(&self) -> String {
        match self {
            StorageConfig::Local(c) => serde_json::to_string(c).unwrap(),
            StorageConfig::NFS(c) => serde_json::to_string(c).unwrap(),
            StorageConfig::S3(c) => serde_json::to_string(c).unwrap(),
            StorageConfig::SFTP(c) => serde_json::to_string(c).unwrap(),
            StorageConfig::FC(c) => serde_json::to_string(c).unwrap(),
            StorageConfig::ZFS(c) => serde_json::to_string(c).unwrap(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalConfig {
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NFSConfig {
    pub host: String,
    pub export_path: String,
    pub mount_options: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct S3Config {
    pub endpoint: String,
    pub bucket: String,
    pub region: String,
    pub access_key: String,
    pub secret_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SFTPConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_type: String,
    pub password: Option<String>,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FCConfig {
    pub target_wwn: String,
    pub lun: i32,
    pub device: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZFSConfig {
    pub pool: String,
    pub dataset: String,
    pub compression: String,
    pub dedup: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CompressionType {
    None,
    Gzip,
    Zstd,
    Lz4,
}

impl std::fmt::Display for CompressionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompressionType::None => write!(f, "none"),
            CompressionType::Gzip => write!(f, "gzip"),
            CompressionType::Zstd => write!(f, "zstd"),
            CompressionType::Lz4 => write!(f, "lz4"),
        }
    }
}

impl CompressionType {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "none" => Some(CompressionType::None),
            "gzip" => Some(CompressionType::Gzip),
            "zstd" => Some(CompressionType::Zstd),
            "lz4" => Some(CompressionType::Lz4),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StorageStatus {
    Connected,
    Disconnected,
    Error,
    Unknown,
}

impl std::fmt::Display for StorageStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StorageStatus::Connected => write!(f, "connected"),
            StorageStatus::Disconnected => write!(f, "disconnected"),
            StorageStatus::Error => write!(f, "error"),
            StorageStatus::Unknown => write!(f, "unknown"),
        }
    }
}

// ── Retention Policy ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetentionPolicy {
    pub keep_last: i32,
    pub keep_hourly: i32,
    pub keep_daily: i32,
    pub keep_weekly: i32,
    pub keep_monthly: i32,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        RetentionPolicy {
            keep_last: 7,
            keep_hourly: 24,
            keep_daily: 7,
            keep_weekly: 4,
            keep_monthly: 12,
        }
    }
}

// ── Dashboard ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardStats {
    pub total_agents: i64,
    pub online_agents: i64,
    pub total_backups: i64,
    pub successful_backups: i64,
    pub failed_backups: i64,
    pub total_databases: i64,
    pub total_storage: i64,
    pub storage_used_bytes: i64,
    pub last_backup_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedisHealth {
    pub ping_ms: f64,
    pub version: String,
    pub uptime_seconds: i64,
    pub used_memory: i64,
    pub used_memory_human: String,
    pub connected_clients: i64,
    pub ops_per_sec: i64,
    pub hit_rate: Option<f64>,
    pub total_keys: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub exp: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub username: String,
    pub password_hash: String,
    pub role: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub ts: i64,
    pub action: String,
    pub entity_type: String,
    pub entity_id: String,
    pub details: String,
    pub user: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsSnapshot {
    pub ts: i64,
    pub cpu_usage: f32,
    pub memory_used: u64,
    pub memory_total: u64,
    pub memory_percent: f32,
    #[serde(default)]
    pub disk_used: u64,
    #[serde(default)]
    pub disk_total: u64,
    #[serde(default)]
    pub disk_percent: f32,
    pub redis_ping_ms: f64,
    #[serde(default)]
    pub redis_memory: i64,
    #[serde(default)]
    pub redis_clients: i64,
    #[serde(default)]
    pub redis_ops_per_sec: i64,
    #[serde(default)]
    pub redis_hit_rate: Option<f64>,
    pub total_backups: i64,
    pub total_agents: i64,
    pub total_storage: i64,
}
