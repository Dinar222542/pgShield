use std::path::PathBuf;

use chrono::{DateTime, Utc};
use redis::aio::ConnectionManager;
use redis::AsyncCommands;
use tracing::info;
use uuid::Uuid;

use pgshield_common::*;

#[derive(Clone)]
pub struct Database {
    conn: ConnectionManager,
    pub backup_dir: String,
}

impl Database {
    pub async fn new(redis_url: &str, backup_dir: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let client = redis::Client::open(redis_url)?;
        let conn = ConnectionManager::new(client).await?;
        info!("Connected to Redis at {redis_url}");
        Ok(Database { conn, backup_dir: backup_dir.into() })
    }

    pub async fn seed(&self) {
        let mut conn = self.conn.clone();
        let exists: bool = conn.exists("idx:agents").await.unwrap_or(false);
        if exists {
            let count = self.scard("idx:agents").await;
            if count > 0 {
                info!("Database already seeded, skipping");
                return;
            }
        }
        info!("Seeding database");

        // Initialize empty index sets so SMEMBERS/SCARD don't cause Redis keyspace misses.
        // Sentinel member `_` is filtered out in list_* functions.
        for key in &["idx:agents", "idx:storages", "idx:backups", "idx:schedules"] {
            let _: Result<(), _> = conn.sadd(key, "_").await;
        }
        // Initialize metrics TTL config so GET config:metrics_ttl doesn't miss
        let _: Result<(), _> = redis::cmd("SET").arg("config:metrics_ttl").arg("30").arg("NX").query_async(&mut conn).await;
    }

    // ── Helpers ──

    async fn get_json<T: serde::de::DeserializeOwned>(&self, key: &str) -> Option<T> {
        let mut conn = self.conn.clone();
        let data: Option<String> = conn.get(key).await.ok().flatten();
        data.and_then(|s| serde_json::from_str(&s).ok())
    }

    async fn set_json<T: serde::Serialize>(&self, key: &str, value: &T) {
        let mut conn = self.conn.clone();
        let json = serde_json::to_string(value).unwrap();
        let _: Result<(), _> = conn.set(key, &json).await;
    }

    async fn del(&self, key: &str) {
        let mut conn = self.conn.clone();
        let _: Result<(), _> = conn.del(key).await;
    }

    async fn exists(&self, key: &str) -> bool {
        let mut conn = self.conn.clone();
        conn.exists(key).await.unwrap_or(false)
    }

    async fn sadd(&self, key: &str, member: &str) {
        let mut conn = self.conn.clone();
        let _: Result<(), _> = conn.sadd(key, member).await;
    }

    async fn srem(&self, key: &str, member: &str) {
        let mut conn = self.conn.clone();
        let _: Result<(), _> = conn.srem(key, member).await;
    }

    async fn smembers(&self, key: &str) -> Vec<String> {
        let mut conn = self.conn.clone();
        let r: Vec<String> = conn.smembers(key).await.unwrap_or_default();
        r.into_iter().filter(|id| id != "_").collect()
    }

    async fn scard(&self, key: &str) -> i64 {
        let count = self.smembers(key).await.len() as i64;
        count
    }

    async fn mget_json<T: serde::de::DeserializeOwned>(&self, keys: &[String]) -> Vec<T> {
        if keys.is_empty() {
            return vec![];
        }
        let mut conn = self.conn.clone();
        let values: Vec<Option<String>> = conn.mget(keys).await.unwrap_or_default();
        values.iter()
            .filter_map(|v| v.as_ref().and_then(|s| serde_json::from_str(s).ok()))
            .collect()
    }

    // ── Dashboard ──

    pub async fn get_dashboard_stats(&self) -> DashboardStats {
        let total_agents = self.scard("idx:agents").await;
        let total_backups = self.scard("idx:backups").await;
        let total_storage = self.scard("idx:storages").await;
        let mut online_agents: i64 = 0;
        let mut successful_backups: i64 = 0;
        let mut failed_backups: i64 = 0;
        let mut storage_used_bytes: i64 = 0;

        let agent_ids = self.smembers("idx:agents").await;
        let backup_ids = self.smembers("idx:backups").await;

        for id in &agent_ids {
            if let Some(a) = self.get_json::<Agent>(&format!("agent:{id}")).await {
                if matches!(a.status, AgentStatus::Online) {
                    online_agents += 1;
                }
            }
        }

        let mut last: Option<DateTime<Utc>> = None;
        let mut dbs: std::collections::HashSet<String> = std::collections::HashSet::new();
        for id in &backup_ids {
            if let Some(b) = self.get_json::<Backup>(&format!("backup:{id}")).await {
                dbs.insert(b.database_name.clone());
                match b.status {
                    BackupStatus::Completed => successful_backups += 1,
                    BackupStatus::Failed => failed_backups += 1,
                    _ => {}
                }
                if let Some(sz) = b.file_size {
                    storage_used_bytes += sz;
                }
                if last.as_ref().map_or(true, |l| b.created_at > *l) {
                    last = Some(b.created_at);
                }
            }
        }
        let total_databases = dbs.len() as i64;

        DashboardStats {
            total_agents,
            online_agents,
            total_backups,
            successful_backups,
            failed_backups,
            total_databases,
            total_storage,
            storage_used_bytes,
            last_backup_at: last,
        }
    }

    // ── Agents ──

    pub async fn list_agents(&self) -> Vec<Agent> {
        let ids = self.smembers("idx:agents").await;
        let keys: Vec<String> = ids.iter().map(|id| format!("agent:{id}")).collect();
        self.mget_json(&keys).await
    }

    pub async fn get_agent(&self, id: Uuid) -> Option<Agent> {
        self.get_json(&format!("agent:{id}")).await
    }

    pub async fn register_agent(&self, agent: &Agent) -> Result<(), redis::RedisError> {
        self.set_json(&format!("agent:{}", agent.id), agent).await;
        self.sadd("idx:agents", &agent.id.to_string()).await;
        Ok(())
    }

    pub async fn delete_agent(&self, id: Uuid) -> Result<bool, redis::RedisError> {
        let key = format!("agent:{id}");
        let mut conn = self.conn.clone();
        let exists: bool = conn.exists(&key).await?;
        if !exists {
            return Ok(false);
        }
        let backup_ids = self.smembers(&format!("idx:backups:agent:{id}")).await;
        for bid in &backup_ids {
            self.del(&format!("backup:{bid}")).await;
            self.srem("idx:backups", bid).await;
        }
        self.del(&format!("idx:backups:agent:{id}")).await;
        self.del(&key).await;
        self.srem("idx:agents", &id.to_string()).await;
        Ok(true)
    }

    // ── Storage ──

    pub async fn list_storage(&self) -> Vec<StorageBackend> {
        let ids = self.smembers("idx:storages").await;
        let keys: Vec<String> = ids.iter().map(|id| format!("storage:{id}")).collect();
        self.mget_json(&keys).await
    }

    pub async fn get_storage(&self, id: Uuid) -> Option<StorageBackend> {
        self.get_json(&format!("storage:{id}")).await
    }

    pub async fn create_storage(&self, s: &StorageBackend) -> Result<(), redis::RedisError> {
        self.set_json(&format!("storage:{}", s.id), s).await;
        self.sadd("idx:storages", &s.id.to_string()).await;
        Ok(())
    }

    pub async fn update_storage(&self, s: &StorageBackend) -> Result<bool, redis::RedisError> {
        let key = format!("storage:{}", s.id);
        let mut conn = self.conn.clone();
        let exists: bool = conn.exists(&key).await?;
        if !exists {
            return Ok(false);
        }
        self.set_json(&key, s).await;
        Ok(true)
    }

    pub async fn delete_storage(&self, id: Uuid) -> Result<bool, redis::RedisError> {
        let key = format!("storage:{id}");
        let mut conn = self.conn.clone();
        let exists: bool = conn.exists(&key).await?;
        if !exists {
            return Ok(false);
        }
        self.del(&format!("retention:{id}")).await;
        self.del(&key).await;
        self.srem("idx:storages", &id.to_string()).await;
        Ok(true)
    }

    pub async fn test_storage_connection(&self, id: Uuid) -> Result<String, String> {
        let storage = self.get_storage(id).await.ok_or("Storage not found")?;
        match storage.storage_type {
            StorageType::Local => {
                let path = match &storage.config {
                    StorageConfig::Local(c) => &c.path,
                    _ => return Err("Invalid config".into()),
                };
                match std::path::Path::new(path).try_exists() {
                    Ok(true) => Ok(format!("Path exists: {path}")),
                    Ok(false) => Err(format!("Path does not exist: {path}")),
                    Err(e) => Err(format!("Error accessing path: {e}")),
                }
            }
            StorageType::NFS => {
                let cfg = match &storage.config {
                    StorageConfig::NFS(c) => c,
                    _ => return Err("Invalid config".into()),
                };
                use std::net::{TcpStream, ToSocketAddrs};
                use std::time::Duration;

                let addr = format!("{}:2049", cfg.host);
                let socket_addrs = match addr.to_socket_addrs() {
                    Ok(a) => a,
                    Err(e) => {
                        let mut updated = storage.clone();
                        updated.status = StorageStatus::Error;
                        updated.updated_at = chrono::Utc::now();
                        let _ = self.update_storage(&updated).await;
                        return Err(format!("DNS resolution failed: {e}"));
                    }
                };
                let sock_addr = match socket_addrs.into_iter().next() {
                    Some(a) => a,
                    None => {
                        let mut updated = storage.clone();
                        updated.status = StorageStatus::Error;
                        updated.updated_at = chrono::Utc::now();
                        let _ = self.update_storage(&updated).await;
                        return Err("Could not resolve host".into());
                    }
                };
                if let Err(e) = TcpStream::connect_timeout(&sock_addr, Duration::from_secs(5)) {
                    let mut updated = storage.clone();
                    updated.status = StorageStatus::Error;
                    updated.updated_at = chrono::Utc::now();
                    let _ = self.update_storage(&updated).await;
                    return Err(format!("NFS недоступен на {}:2049 — {e}", cfg.host));
                }

                let mut details = format!("NFS port 2049 доступен на {}", cfg.host);

                // showmount -e
                if let Ok(out) = std::process::Command::new("showmount")
                    .args(["-e", &cfg.host])
                    .output()
                {
                    if out.status.success() {
                        let stdout = String::from_utf8_lossy(&out.stdout);
                        let trimmed = stdout.trim();
                        if !trimmed.is_empty() {
                            details.push_str(&format!("\nЭкспорты:\n{}", trimmed));
                        }
                    }
                }

                // Try to mount and get real disk usage
                let mountpoint = format!("/mnt/pgshield-{}", id);
                let export = format!("{}:{}", cfg.host, cfg.export_path);
                let _ = std::fs::create_dir_all(&mountpoint);

                // Start rpcbind if available (needed for some NFS versions)
                let _ = std::process::Command::new("rpcbind").output();

                let mount_opts = if cfg.mount_options.is_empty() {
                    "soft,timeo=10"
                } else {
                    &cfg.mount_options
                };

                let mount_result = std::process::Command::new("mount")
                    .args(["-t", "nfs", "-o", mount_opts, &export, &mountpoint])
                    .output();

                match mount_result {
                    Ok(out) if out.status.success() => {
                        if let Ok(df) = std::process::Command::new("df")
                            .args(["--output=size,used", &mountpoint]).output()
                        {
                            if df.status.success() {
                                let stdout = String::from_utf8_lossy(&df.stdout);
                                let line = stdout.lines().nth(1).unwrap_or("");
                                let parts: Vec<&str> = line.split_whitespace().collect();
                                if parts.len() >= 2 {
                                    let total_kb: u64 = parts[0].parse().unwrap_or(0);
                                    let used_kb: u64 = parts[1].parse().unwrap_or(0);
                                    if total_kb > 0 {
                                        let total_bytes = (total_kb as i64) * 1024;
                                        let used_bytes = (used_kb as i64) * 1024;
                                        let free_pct = (total_kb - used_kb) * 100 / total_kb;
                                        let mut updated = storage.clone();
                                        updated.total_space = Some(total_bytes);
                                        updated.used_space = Some(used_bytes);
                                        updated.status = StorageStatus::Connected;
                                        updated.updated_at = chrono::Utc::now();
                                        let _ = self.update_storage(&updated).await;
                                        details.push_str(&format!(
                                            "\n✅ Смонтирован успешно\n  Alloc: {} / {} ({}% свободно)",
                                            fmt_bytes(used_bytes), fmt_bytes(total_bytes), free_pct
                                        ));
                                    }
                                }
                            }
                        }
                        let _ = std::process::Command::new("umount").arg("-l").arg(&mountpoint).output();
                        let _ = std::fs::remove_dir(&mountpoint);
                    }
                    Ok(out) => {
                        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
                        let _ = std::fs::remove_dir(&mountpoint);

                        // Fallback: try bind-mounted path
                        let bind_path = format!("/mnt/nfs-pgshield");
                        if std::path::Path::new(&bind_path).exists() {
                            if let Ok(df) = std::process::Command::new("df")
                                .args(["--output=size,used", &bind_path]).output()
                            {
                                if df.status.success() {
                                    let stdout = String::from_utf8_lossy(&df.stdout);
                                    let line = stdout.lines().nth(1).unwrap_or("");
                                    let parts: Vec<&str> = line.split_whitespace().collect();
                                    if parts.len() >= 2 {
                                        let total_kb: u64 = parts[0].parse().unwrap_or(0);
                                        let used_kb: u64 = parts[1].parse().unwrap_or(0);
                                        if total_kb > 0 {
                                            let total_bytes = (total_kb as i64) * 1024;
                                            let used_bytes = (used_kb as i64) * 1024;
                                            let free_pct = (total_kb - used_kb) * 100 / total_kb;
                                            let mut updated = storage.clone();
                                            updated.total_space = Some(total_bytes);
                                            updated.used_space = Some(used_bytes);
                                            updated.status = StorageStatus::Connected;
                                            updated.updated_at = chrono::Utc::now();
                                            let _ = self.update_storage(&updated).await;
                                            details.push_str(&format!(
                                                "\n✅ NFS смонтирован на хосте (bind mount)\n  Alloc: {} / {} ({}% свободно)",
                                                fmt_bytes(used_bytes), fmt_bytes(total_bytes), free_pct
                                            ));
                                            return Ok(details);
                                        }
                                    }
                                }
                            }
                        }

                        let msg = if stderr.contains("Operation not permitted") {
                            "Сервер NFS отклонил подключение. Проверьте /etc/exports — добавьте IP Podman VM в список разрешённых.".into()
                        } else if stderr.contains("No such file or directory") {
                            format!("Экспорт '{}' не найден на сервере. Доступные экспорты показаны выше.", cfg.export_path)
                        } else {
                            format!("Ошибка монтирования: {}", stderr)
                        };
                        details.push_str(&format!("\n⚠️  {}", msg));
                        let mut updated = storage.clone();
                        updated.status = StorageStatus::Error;
                        updated.updated_at = chrono::Utc::now();
                        let _ = self.update_storage(&updated).await;
                    }
                    Err(e) => {
                        details.push_str(&format!("\n⚠️  mount error: {e}"));
                        let _ = std::fs::remove_dir(&mountpoint);
                        let mut updated = storage.clone();
                        updated.status = StorageStatus::Error;
                        updated.updated_at = chrono::Utc::now();
                        let _ = self.update_storage(&updated).await;
                    }
                }

                Ok(details)
            }
            _ => Err("Unsupported storage type".into()),
        }
    }

    // ── Backups ──

    pub async fn list_backups(&self, agent_id: Option<Uuid>, storage_id: Option<Uuid>) -> Vec<Backup> {
        let ids: Vec<String> = match (agent_id, storage_id) {
            (Some(aid), _) => self.smembers(&format!("idx:backups:agent:{aid}")).await,
            (_, Some(sid)) => self.smembers(&format!("idx:backups:storage:{sid}")).await,
            _ => self.smembers("idx:backups").await,
        };
        let keys: Vec<String> = ids.iter().map(|id| format!("backup:{id}")).collect();

        let mut result: Vec<Backup> = self.mget_json(&keys).await;
        result.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        result
    }

    pub async fn get_backup(&self, id: Uuid) -> Option<Backup> {
        self.get_json(&format!("backup:{id}")).await
    }

    pub async fn create_backup(&self, backup: &Backup) -> Result<(), redis::RedisError> {
        self.set_json(&format!("backup:{}", backup.id), backup).await;
        self.sadd("idx:backups", &backup.id.to_string()).await;
        self.sadd(&format!("idx:backups:agent:{}", backup.agent_id), &backup.id.to_string()).await;
        if let Some(sid) = backup.storage_id {
            self.sadd(&format!("idx:backups:storage:{sid}"), &backup.id.to_string()).await;
        }
        Ok(())
    }

    pub async fn delete_backup(&self, id: Uuid) -> Result<bool, redis::RedisError> {
        let key = format!("backup:{id}");
        let mut conn = self.conn.clone();
        let exists: bool = conn.exists(&key).await?;
        if !exists {
            return Ok(false);
        }
        if let Some(b) = self.get_backup(id).await {
            self.srem(&format!("idx:backups:agent:{}", b.agent_id), &id.to_string()).await;
            if let Some(sid) = b.storage_id {
                self.srem(&format!("idx:backups:storage:{sid}"), &id.to_string()).await;
            }
        }
        self.srem("idx:backups", &id.to_string()).await;
        self.del(&key).await;
        Ok(true)
    }

    // ── Retention ──

    pub async fn get_retention_policy(&self, storage_id: Uuid) -> Option<RetentionPolicy> {
        self.get_json(&format!("retention:{storage_id}")).await
    }

    pub async fn upsert_retention_policy(&self, storage_id: Uuid, policy: &RetentionPolicy) -> Result<(), redis::RedisError> {
        self.set_json(&format!("retention:{storage_id}"), policy).await;
        Ok(())
    }

    // ── Metrics History ──

    pub async fn push_metrics(&self, snapshot: &MetricsSnapshot) {
        let json = serde_json::to_string(snapshot).unwrap();
        let mut conn = self.conn.clone();
        let ts = snapshot.ts as f64;
        let _: Result<(), _> = conn.zadd("metrics", &json, ts).await;
        let ttl = self.get_metrics_ttl().await;
        let cutoff = ts - (ttl as f64 * 86400.0);
        let _: Result<(), _> = redis::cmd("ZREMRANGEBYSCORE").arg("metrics").arg(0).arg(cutoff).query_async(&mut conn).await;
        let _: Result<(), _> = conn.expire("metrics", ttl * 86400).await;
    }

    pub async fn get_metrics_history(&self, range_secs: i64) -> Vec<MetricsSnapshot> {
        let now = chrono::Utc::now().timestamp();
        let min = (now - range_secs) as f64;
        let mut conn = self.conn.clone();
        let results: Vec<String> = conn.zrangebyscore("metrics", min, now as f64).await.unwrap_or_default();
        results.iter().filter_map(|s| serde_json::from_str(s).ok()).collect()
    }

    // ── Redis Health ──

    pub async fn redis_health(&self) -> RedisHealth {
        let mut conn = self.conn.clone();

        // Ping latency
        let ping_start = std::time::Instant::now();
        let _: Result<String, _> = redis::cmd("PING").query_async(&mut conn).await;
        let ping_ms = ping_start.elapsed().as_secs_f64() * 1000.0;

        // INFO
        let info_raw: String = redis::cmd("INFO").query_async(&mut conn).await.unwrap_or_default();

        // DBSIZE
        let total_keys: i64 = redis::cmd("DBSIZE").query_async(&mut conn).await.unwrap_or(0);

        parse_redis_info(&info_raw, ping_ms, total_keys)
    }

    // ── Audit Log ──

    pub async fn log_audit(&self, action: &str, entity_type: &str, entity_id: &str, details: &str, user: &str) {
        let entry = AuditEntry {
            ts: chrono::Utc::now().timestamp(),
            action: action.to_string(),
            entity_type: entity_type.to_string(),
            entity_id: entity_id.to_string(),
            details: details.to_string(),
            user: user.to_string(),
        };
        let json = serde_json::to_string(&entry).unwrap();
        let mut conn = self.conn.clone();
        let ts = entry.ts as f64;
        let _: Result<(), _> = conn.zadd("audit", &json, ts).await;
        let cutoff = ts - 90.0 * 86400.0;
        let _: Result<(), _> = redis::cmd("ZREMRANGEBYSCORE").arg("audit").arg(0).arg(cutoff).query_async(&mut conn).await;
        let _: Result<(), _> = conn.expire("audit", 90 * 86400).await;
    }

    pub async fn get_audit_logs(&self, limit: usize, offset: usize) -> Vec<AuditEntry> {
        let mut conn = self.conn.clone();
        let total: Vec<String> = conn
            .zrevrange("audit", offset as isize, (offset + limit - 1) as isize)
            .await
            .unwrap_or_default();
        total.iter().filter_map(|s| serde_json::from_str(s).ok()).collect()
    }

    // ── Metrics Config ──

    pub async fn get_metrics_ttl(&self) -> i64 {
        let mut conn = self.conn.clone();
        let val: Option<String> = conn.get("config:metrics_ttl").await.unwrap_or(None);
        val.and_then(|v| v.parse().ok()).unwrap_or(30)
    }

    pub async fn set_metrics_ttl(&self, days: i64) {
        let _: Result<(), _> = self.conn.clone().set("config:metrics_ttl", days.to_string()).await;
    }

    pub async fn clear_metrics_history(&self) {
        let _: Result<(), _> = self.conn.clone().del("metrics").await;
    }

    // ── User Management ──

    pub async fn create_user(&self, user: &User) -> Result<(), redis::RedisError> {
        let key = format!("user:{}", user.username);
        self.set_json(&key, user).await;
        let _: Result<(), _> = self.conn.clone().sadd("idx:users", &user.username).await;
        Ok(())
    }

    pub async fn get_user(&self, username: &str) -> Option<User> {
        self.get_json(&format!("user:{username}")).await
    }

    pub async fn list_users(&self) -> Vec<User> {
        let mut conn = self.conn.clone();
        let usernames: Vec<String> = conn.smembers("idx:users").await.unwrap_or_default();
        let mut users = Vec::new();
        for u in usernames {
            if let Some(user) = self.get_user(&u).await {
                users.push(user);
            }
        }
        users
    }

    pub async fn delete_user(&self, username: &str) -> bool {
        let key = format!("user:{username}");
        let existed = self.exists(&key).await;
        self.del(&key).await;
        let _: Result<(), _> = self.conn.clone().srem("idx:users", username).await;
        existed
    }

    pub async fn authenticate_user(&self, username: &str, password: &str) -> Option<User> {
        let user = self.get_user(username).await?;
        if bcrypt::verify(password, &user.password_hash).unwrap_or(false) {
            Some(user)
        } else {
            None
        }
    }

    pub async fn seed_user(&self, username: &str, password: &str) {
        if self.get_user(username).await.is_some() {
            return;
        }
        let hash = bcrypt::hash(password, 4).unwrap_or_else(|_| password.into());
        let user = User {
            username: username.to_string(),
            password_hash: hash,
            role: "admin".into(),
            created_at: chrono::Utc::now().timestamp(),
        };
        let _ = self.create_user(&user).await;
        info!("Seeded user: {username}");
    }

    // ── BackupSchedule CRUD ──

    pub async fn list_schedules(&self) -> Vec<BackupSchedule> {
        let ids = self.smembers("idx:schedules").await;
        let keys: Vec<String> = ids.iter().map(|id| format!("schedule:{id}")).collect();
        self.mget_json(&keys).await
    }

    pub async fn get_schedule(&self, id: Uuid) -> Option<BackupSchedule> {
        self.get_json(&format!("schedule:{id}")).await
    }

    pub async fn create_schedule(&self, s: &BackupSchedule) {
        self.set_json(&format!("schedule:{}", s.id), s).await;
        self.sadd("idx:schedules", &s.id.to_string()).await;
    }

    pub async fn update_schedule(&self, s: &BackupSchedule) -> bool {
        let key = format!("schedule:{}", s.id);
        if !self.exists(&key).await {
            return false;
        }
        self.set_json(&key, s).await;
        true
    }

    pub async fn delete_schedule(&self, id: Uuid) -> bool {
        let key = format!("schedule:{id}");
        let existed = self.exists(&key).await;
        self.del(&key).await;
        self.srem("idx:schedules", &id.to_string()).await;
        existed
    }

    // ── Backup Execution (local pg_dump) ──

    pub async fn run_backup_local(&self, schedule: &BackupSchedule) -> Result<(Uuid, String, i64, Option<String>), String> {
        let backup_id = Uuid::new_v4();
        let ts = Utc::now().format("%Y%m%d_%H%M%S");
        let filename = format!("{}_{}_{}.dump", schedule.database, ts, backup_id);
        let file_path = PathBuf::from(&self.backup_dir).join(&filename);

        std::fs::create_dir_all(&self.backup_dir).map_err(|e| format!("Cannot create backup dir: {e}"))?;

        tracing::info!("Running local backup: {} -> {}", schedule.database, file_path.display());

        let output = std::process::Command::new("pg_dump")
            .env("PGPASSWORD", &schedule.db_password)
            .args([
                "-h", &schedule.db_host,
                "-p", &schedule.db_port.to_string(),
                "-U", &schedule.db_user,
                "-F", "c",
                "-f", file_path.to_str().unwrap(),
                &schedule.database,
            ])
            .output()
            .map_err(|e| format!("Failed to execute pg_dump: {e}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("pg_dump failed: {}", stderr.trim()));
        }

        let metadata = std::fs::metadata(&file_path).map_err(|e| format!("Cannot read backup file: {e}"))?;
        let file_size = metadata.len() as i64;

        let checksum = compute_sha256(&file_path).ok();

        Ok((backup_id, file_path.to_str().unwrap().to_string(), file_size, checksum))
    }

    pub async fn run_restore_local(
        &self,
        file_path: &str,
        target_db: &str,
        target_host: &str,
        target_port: u16,
        target_user: &str,
        target_password: &str,
    ) -> Result<String, String> {
        let path = PathBuf::from(file_path);
        if !path.exists() {
            return Err(format!("Backup file not found: {file_path}"));
        }

        tracing::info!("Running local restore: {} -> {}@{}/{}", file_path, target_db, target_host, target_db);

        let output = std::process::Command::new("pg_restore")
            .env("PGPASSWORD", target_password)
            .args([
                "-h", target_host,
                "-p", &target_port.to_string(),
                "-U", target_user,
                "-d", target_db,
                "-c",
                "--if-exists",
                "-v",
                file_path,
            ])
            .output()
            .map_err(|e| format!("Failed to execute pg_restore: {e}"))?;

        if output.status.success() {
            Ok("Restore completed successfully".into())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(format!("Restore failed: {}", stderr.trim()))
        }
    }

    // ── Agent communication ──

    pub async fn call_agent_backup(&self, agent: &Agent, db: &str, host: &str, port: u16, user: &str, password: &str) -> Result<(Uuid, String, i64, Option<String>), String> {
        let url = format!("http://{}:{}/api/v1/backup", agent.host, agent.port);
        let client = reqwest::Client::new();
        let body = serde_json::json!({
            "database": db,
            "db_host": host,
            "db_port": port,
            "db_user": user,
            "db_password": password,
        });

        let resp = client.post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Agent unreachable: {e}"))?;

        let result: serde_json::Value = resp.json().await.map_err(|e| format!("Bad response: {e}"))?;

        if result["success"].as_bool().unwrap_or(false) {
            let backup_id = Uuid::new_v4();
            let file_path = result["file_path"].as_str().unwrap_or("").to_string();
            let file_size = result["file_size"].as_i64().unwrap_or(0);
            let checksum = result["checksum"].as_str().map(|s| s.to_string());
            Ok((backup_id, file_path, file_size, checksum))
        } else {
            Err(result["message"].as_str().unwrap_or("Unknown error").to_string())
        }
    }

    pub async fn call_agent_restore(
        &self,
        agent: &Agent,
        backup_path: &str,
        target_db: &str,
        target_host: &str,
        target_port: u16,
        target_user: &str,
        target_password: &str,
    ) -> Result<String, String> {
        let url = format!("http://{}:{}/api/v1/restore", agent.host, agent.port);
        let client = reqwest::Client::new();
        let body = serde_json::json!({
            "backup_path": backup_path,
            "target_db": target_db,
            "target_host": target_host,
            "target_port": target_port,
            "target_user": target_user,
            "target_password": target_password,
        });

        let resp = client.post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Agent unreachable: {e}"))?;

        let result: serde_json::Value = resp.json().await.map_err(|e| format!("Bad response: {e}"))?;

        if result["success"].as_bool().unwrap_or(false) {
            Ok(result["message"].as_str().unwrap_or("Restore completed").to_string())
        } else {
            Err(result["message"].as_str().unwrap_or("Unknown error").to_string())
        }
    }
}

fn compute_sha256(path: &PathBuf) -> Result<String, std::io::Error> {
    use std::io::Read;
    use sha2::Digest;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = sha2::Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn parse_redis_info(raw: &str, ping_ms: f64, total_keys: i64) -> RedisHealth {
    let mut version = String::new();
    let mut uptime_seconds: i64 = 0;
    let mut used_memory: i64 = 0;
    let mut used_memory_human = String::new();
    let mut connected_clients: i64 = 0;
    let mut ops_per_sec: i64 = 0;
    let mut keyspace_hits: i64 = 0;
    let mut keyspace_misses: i64 = 0;

    for line in raw.lines() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if let Some((key, val)) = line.split_once(':') {
            match key {
                "redis_version" => version = val.to_string(),
                "uptime_in_seconds" => uptime_seconds = val.parse().unwrap_or(0),
                "used_memory" => used_memory = val.parse().unwrap_or(0),
                "used_memory_human" => used_memory_human = val.to_string(),
                "connected_clients" => connected_clients = val.parse().unwrap_or(0),
                "instantaneous_ops_per_sec" => ops_per_sec = val.parse().unwrap_or(0),
                "keyspace_hits" => keyspace_hits = val.parse().unwrap_or(0),
                "keyspace_misses" => keyspace_misses = val.parse().unwrap_or(0),
                _ => {}
            }
        }
    }

    let hit_rate = if keyspace_hits + keyspace_misses > 0 {
        Some(keyspace_hits as f64 / (keyspace_hits + keyspace_misses) as f64 * 100.0)
    } else {
        None
    };

    RedisHealth {
        ping_ms,
        version,
        uptime_seconds,
        used_memory,
        used_memory_human,
        connected_clients,
        ops_per_sec,
        hit_rate,
        total_keys,
    }
}

fn fmt_bytes(bytes: i64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    for u in UNITS {
        if v < 1024.0 {
            return format!("{:.1}{}", v, u);
        }
        v /= 1024.0;
    }
    format!("{:.1}{}", v * 1024.0, UNITS[UNITS.len() - 1])
}
