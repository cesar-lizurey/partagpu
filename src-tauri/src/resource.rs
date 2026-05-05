use serde::{Deserialize, Serialize};
use std::process::Command;
use sysinfo::System;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ResourceUsage {
    pub cpu_percent: f32,
    pub cpu_cores: usize,
    pub ram_used_mb: u64,
    pub ram_total_mb: u64,
    pub ram_percent: f32,
    pub gpu_percent: f32,
    pub gpu_name: String,
    pub gpu_memory_used_mb: u64,
    pub gpu_memory_total_mb: u64,
    pub gpu_available: bool,
    /// Total number of CUDA devices visible to nvidia-smi on this host.
    /// 0 when no NVIDIA driver is loaded.
    pub gpu_count: u32,
    /// Per-device snapshot. Empty when no GPU is available.
    pub gpus: Vec<GpuDevice>,
}

/// Snapshot of one CUDA device.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GpuDevice {
    pub index: u32,
    pub name: String,
    pub utilization: f32,
    pub memory_used_mb: u64,
    pub memory_total_mb: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GpuInfo {
    pub name: String,
    pub utilization: f32,
    pub memory_used_mb: u64,
    pub memory_total_mb: u64,
    pub available: bool,
}

pub struct ResourceMonitor {
    sys: System,
}

impl ResourceMonitor {
    pub fn new() -> Self {
        let mut sys = System::new_all();
        sys.refresh_all();
        Self { sys }
    }

    pub fn snapshot(&mut self) -> ResourceUsage {
        self.sys.refresh_all();
        std::thread::sleep(std::time::Duration::from_millis(200));
        self.sys.refresh_cpu_usage();

        let cpu_percent = self.sys.global_cpu_usage();
        let cpu_cores = self.sys.cpus().len();
        let ram_total_mb = self.sys.total_memory() / (1024 * 1024);
        let ram_used_mb = self.sys.used_memory() / (1024 * 1024);
        let ram_percent = if ram_total_mb > 0 {
            (ram_used_mb as f32 / ram_total_mb as f32) * 100.0
        } else {
            0.0
        };

        let gpus = list_gpus();
        let gpu_count = gpus.len() as u32;
        // Aggregate view for the legacy single-GPU UI gauge: take device 0 if any.
        let primary = gpus.first().cloned().unwrap_or_default();
        let gpu_available = !gpus.is_empty();

        ResourceUsage {
            cpu_percent,
            cpu_cores,
            ram_used_mb,
            ram_total_mb,
            ram_percent,
            gpu_percent: primary.utilization,
            gpu_name: primary.name.clone(),
            gpu_memory_used_mb: primary.memory_used_mb,
            gpu_memory_total_mb: primary.memory_total_mb,
            gpu_available,
            gpu_count,
            gpus,
        }
    }
}

/// Return the list of CUDA devices visible to nvidia-smi, in index order.
/// Empty when nvidia-smi is missing, fails, or there is no NVIDIA driver.
///
/// The `PARTAGPU_FORCE_GPU_COUNT` env var lets you simulate N GPUs on a
/// single-GPU host for testing the multi-GPU dispatch logic. When set, the
/// real device-0 snapshot is duplicated N times with synthetic indices.
pub fn list_gpus() -> Vec<GpuDevice> {
    let real = real_gpus();

    if let Ok(s) = std::env::var("PARTAGPU_FORCE_GPU_COUNT") {
        if let Ok(n) = s.parse::<u32>() {
            if n > 0 {
                let template = real.first().cloned().unwrap_or_else(|| GpuDevice {
                    index: 0,
                    name: "synthetic-test-gpu".to_string(),
                    utilization: 0.0,
                    memory_used_mb: 0,
                    memory_total_mb: 0,
                });
                return (0..n)
                    .map(|i| GpuDevice {
                        index: i,
                        ..template.clone()
                    })
                    .collect();
            }
        }
    }

    real
}

fn real_gpus() -> Vec<GpuDevice> {
    let output = Command::new("nvidia-smi")
        .args([
            "--query-gpu=index,name,utilization.gpu,memory.used,memory.total",
            "--format=csv,noheader,nounits",
        ])
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            stdout
                .lines()
                .filter_map(|line| {
                    let parts: Vec<&str> = line.split(", ").collect();
                    if parts.len() < 5 {
                        return None;
                    }
                    Some(GpuDevice {
                        index: parts[0].trim().parse().ok()?,
                        name: parts[1].trim().to_string(),
                        utilization: parts[2].trim().parse().unwrap_or(0.0),
                        memory_used_mb: parts[3].trim().parse().unwrap_or(0),
                        memory_total_mb: parts[4].trim().parse().unwrap_or(0),
                    })
                })
                .collect()
        }
        _ => Vec::new(),
    }
}
