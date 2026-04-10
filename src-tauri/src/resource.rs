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

        let gpu = query_gpu();

        ResourceUsage {
            cpu_percent,
            cpu_cores,
            ram_used_mb,
            ram_total_mb,
            ram_percent,
            gpu_percent: gpu.utilization,
            gpu_name: gpu.name,
            gpu_memory_used_mb: gpu.memory_used_mb,
            gpu_memory_total_mb: gpu.memory_total_mb,
            gpu_available: gpu.available,
        }
    }
}

fn query_gpu() -> GpuInfo {
    let output = Command::new("nvidia-smi")
        .args([
            "--query-gpu=name,utilization.gpu,memory.used,memory.total",
            "--format=csv,noheader,nounits",
        ])
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let line = stdout.trim();
            let parts: Vec<&str> = line.split(", ").collect();
            if parts.len() >= 4 {
                GpuInfo {
                    name: parts[0].to_string(),
                    utilization: parts[1].parse().unwrap_or(0.0),
                    memory_used_mb: parts[2].parse().unwrap_or(0),
                    memory_total_mb: parts[3].parse().unwrap_or(0),
                    available: true,
                }
            } else {
                GpuInfo::default()
            }
        }
        _ => GpuInfo::default(),
    }
}
