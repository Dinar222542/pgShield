use serde::Serialize;
use sysinfo::{Disks, System};

#[derive(Debug, Clone, Serialize, Default)]
pub struct SystemInfo {
    pub hostname: String,
    pub kernel: String,
    pub uptime_seconds: u64,
    pub cpu_usage: f32,
    pub memory_total: u64,
    pub memory_used: u64,
    pub memory_percent: f32,
    pub disk_total: u64,
    pub disk_used: u64,
    pub disk_percent: f32,
}

pub fn gather() -> SystemInfo {
    let mut sys = System::new_all();
    sys.refresh_all();

    let hostname = System::host_name().unwrap_or_default();
    let kernel = System::kernel_version().unwrap_or_default();
    let uptime_seconds = System::uptime();

    let cpu_usage = sys.global_cpu_usage();

    let memory_total = sys.total_memory();
    let memory_used = sys.used_memory();
    let memory_percent = if memory_total > 0 {
        memory_used as f32 / memory_total as f32 * 100.0
    } else {
        0.0
    };

    let disks = Disks::new_with_refreshed_list();
    let mut disk_total: u64 = 0;
    let mut disk_used: u64 = 0;
    for disk in &disks {
        disk_total += disk.total_space();
        disk_used += disk.total_space() - disk.available_space();
    }
    let disk_percent = if disk_total > 0 {
        disk_used as f32 / disk_total as f32 * 100.0
    } else {
        0.0
    };

    SystemInfo {
        hostname,
        kernel,
        uptime_seconds,
        cpu_usage,
        memory_total,
        memory_used,
        memory_percent,
        disk_total,
        disk_used,
        disk_percent,
    }
}
