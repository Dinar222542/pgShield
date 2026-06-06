use std::sync::Arc;

use chrono::Utc;
use uuid::Uuid;
use tokio::sync::Mutex;
use tracing::info;

use crate::db::Database;
use pgshield_common::*;

pub struct Scheduler {
    db: Database,
    running: Arc<Mutex<bool>>,
}

impl Scheduler {
    pub fn new(db: Database) -> Self {
        Scheduler {
            db,
            running: Arc::new(Mutex::new(false)),
        }
    }

    pub async fn start(&self) {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            if let Err(e) = self.tick().await {
                tracing::error!("Scheduler tick error: {e}");
            }
        }
    }

    async fn tick(&self) -> Result<(), String> {
        let schedules = self.db.list_schedules().await;
        let now = Utc::now();

        for sched in &schedules {
            if !sched.enabled {
                continue;
            }

            // Check if this schedule should run now
            if let Some(next) = self.should_run(sched, now) {
                info!("Schedule '{}' firing (next: {})", sched.name, next);
                self.execute_schedule(sched).await;
            }
        }

        Ok(())
    }

    fn should_run(&self, sched: &BackupSchedule, now: chrono::DateTime<Utc>) -> Option<chrono::DateTime<Utc>> {
        let cron = sched.cron_expr.parse::<cron::Schedule>().ok()?;

        // If never run, find next occurrence
        let last = match sched.last_run {
            Some(ts) => {
                let last_time = chrono::DateTime::from_timestamp(ts, 0).unwrap_or(now);
                // Check if we're past the next occurrence after last_run
                let mut upcoming = cron.after(&last_time);
                let next = upcoming.next()?;
                if next > now {
                    return None; // Not time yet
                }
                // Only fire if the next occurrence is within 120 seconds of now
                let diff = (now - next).num_seconds().abs();
                if diff > 120 {
                    return None; // Too far from the scheduled time (avoid catching up)
                }
                last_time
            }
            None => {
                // Never run — find first occurrence
                let mut upcoming = cron.upcoming(Utc).take(1);
                let first = upcoming.next()?;
                if first > now {
                    return None;
                }
                return Some(first);
            }
        };

        let mut upcoming = cron.after(&last);
        let next = upcoming.next()?;
        let diff = (now - next).num_seconds().abs();
        if diff <= 120 {
            Some(next)
        } else {
            None
        }
    }

    async fn execute_schedule(&self, sched: &BackupSchedule) {
        let start = std::time::Instant::now();
        let backup_result = if let Some(agent_id) = sched.agent_id {
            // Try agent
            if let Some(agent) = self.db.get_agent(agent_id).await {
                self.db.call_agent_backup(
                    &agent,
                    &sched.database,
                    &sched.db_host,
                    sched.db_port,
                    &sched.db_user,
                    &sched.db_password,
                ).await
            } else {
                Err("Agent not found".into())
            }
        } else {
            // Local pg_dump
            self.db.run_backup_local(sched).await
        };

        let elapsed = start.elapsed();

        match backup_result {
            Ok((backup_id, file_path, file_size, checksum)) => {
                let backup = Backup {
                    id: backup_id,
                    agent_id: sched.agent_id.unwrap_or_else(Uuid::nil),
                    storage_id: Some(sched.storage_id),
                    database_name: sched.database.clone(),
                    backup_type: BackupType::Full,
                    status: BackupStatus::Completed,
                    file_path: Some(file_path),
                    file_size: Some(file_size),
                    checksum,
                    error_message: None,
                    retention_until: Some(Utc::now() + chrono::Duration::days(sched.retention_days as i64)),
                    created_at: Utc::now(),
                    completed_at: Some(Utc::now()),
                };
                let _ = self.db.create_backup(&backup).await;
                self.db.log_audit(
                    "backup_completed", "backup", &backup_id.to_string(),
                    &format!("Scheduled backup '{}' completed ({} bytes, took {:?})", sched.name, file_size, elapsed),
                    "scheduler",
                ).await;

                let mut updated = sched.clone();
                updated.last_run = Some(Utc::now().timestamp());
                updated.last_status = Some("success".into());
                updated.updated_at = Utc::now();
                self.db.update_schedule(&updated).await;

                info!("Scheduled backup '{}' completed in {:?} ({} bytes)", sched.name, elapsed, file_size);
            }
            Err(e) => {
                self.db.log_audit(
                    "backup_failed", "backup", "",
                    &format!("Scheduled backup '{}' failed: {}", sched.name, e),
                    "scheduler",
                ).await;

                let mut updated = sched.clone();
                updated.last_run = Some(Utc::now().timestamp());
                updated.last_status = Some("failed".into());
                updated.updated_at = Utc::now();
                self.db.update_schedule(&updated).await;

                tracing::error!("Scheduled backup '{}' failed: {} (elapsed: {:?})", sched.name, e, elapsed);
            }
        }
    }
}
