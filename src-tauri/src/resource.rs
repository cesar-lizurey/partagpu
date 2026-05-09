use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use sysinfo::System;

/// Same path as `sandbox::exec::MPS_PIPE_DIR`. Duplicated here (rather than
/// `pub use`d) because the sandbox module is internal and we only need the
/// const + a liveness probe to populate `ResourceUsage::gpu_limit_enforced`.
const MPS_PIPE_DIR: &str = "/var/lib/partagpu/mps";

/// Resource history sample : one tick of CPU / RAM / GPU usage with a
/// unix-epoch timestamp. Compact representation chosen for cheap JSON
/// serialization to the frontend (which renders sparklines).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceSample {
    pub timestamp_secs: u64,
    pub cpu_percent: f32,
    pub ram_used_mb: u64,
    pub gpu_percent: f32,
}

/// Sampling cadence of the background history collector. 5 s gives good
/// resolution without burning CPU on `refresh_all` calls.
const HISTORY_SAMPLE_INTERVAL: Duration = Duration::from_secs(5);

/// How many samples to keep in the ring buffer. 60 × 5 s = 5 minutes.
const HISTORY_MAX_SAMPLES: usize = 60;

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
    /// True when the GPU share-limit slider is actually enforced — i.e. the
    /// CUDA MPS daemon is up so `CUDA_MPS_ACTIVE_THREAD_PERCENTAGE` injected
    /// into the sandbox env is honoured. False when MPS is missing or its
    /// daemon isn't running ; in that case the slider is advisory-only and
    /// the UI shows a warning to set expectations.
    pub gpu_limit_enforced: bool,
}

/// Liveness probe for the CUDA MPS daemon. Mirrors the check in
/// `sandbox::exec::is_mps_daemon_alive` — we duplicate it here so the
/// frontend can be told whether the GPU share-limit is actually applied
/// without having to call into the sandbox module.
fn is_mps_active() -> bool {
    if !Path::new(MPS_PIPE_DIR).is_dir() {
        return false;
    }
    Command::new("pgrep")
        .args(["-x", "nvidia-cuda-mps-control"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
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
    /// Ring buffer of recent CPU/RAM/GPU samples. Filled by a background
    /// collector thread spawned in `new()`. Rendered as sparklines in the UI.
    history: Arc<Mutex<VecDeque<ResourceSample>>>,
}

impl Default for ResourceMonitor {
    fn default() -> Self {
        Self::new()
    }
}

impl ResourceMonitor {
    pub fn new() -> Self {
        let mut sys = System::new_all();
        sys.refresh_all();
        let history = Arc::new(Mutex::new(VecDeque::with_capacity(HISTORY_MAX_SAMPLES)));
        Self::spawn_history_collector(history.clone());
        Self { sys, history }
    }

    /// Background thread that owns its own `sysinfo::System` (since `snapshot`
    /// requires `&mut self`) and pushes one `ResourceSample` to the ring buffer
    /// every `HISTORY_SAMPLE_INTERVAL`. Drops the oldest sample when full.
    fn spawn_history_collector(history: Arc<Mutex<VecDeque<ResourceSample>>>) {
        std::thread::spawn(move || {
            let mut sys = System::new_all();
            sys.refresh_all();
            loop {
                std::thread::sleep(HISTORY_SAMPLE_INTERVAL);
                sys.refresh_all();
                std::thread::sleep(Duration::from_millis(200));
                sys.refresh_cpu_usage();

                let cpu_percent = sys.global_cpu_usage();
                let ram_used_mb = sys.used_memory() / (1024 * 1024);
                let gpu_percent = list_gpus().first().map(|g| g.utilization).unwrap_or(0.0);
                let timestamp_secs = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);

                let sample = ResourceSample {
                    timestamp_secs,
                    cpu_percent,
                    ram_used_mb,
                    gpu_percent,
                };
                let mut h = history.lock().unwrap();
                h.push_back(sample);
                if h.len() > HISTORY_MAX_SAMPLES {
                    h.pop_front();
                }
            }
        });
    }

    /// Snapshot of the resource history ring buffer (oldest → newest).
    /// Cheap clone of a small `VecDeque` — at most `HISTORY_MAX_SAMPLES` items.
    pub fn history(&self) -> Vec<ResourceSample> {
        self.history.lock().unwrap().iter().cloned().collect()
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
            gpu_limit_enforced: gpu_available && is_mps_active(),
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
