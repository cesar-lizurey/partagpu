//! Process-tree resource sampling helpers used by the per-task monitor
//! thread. Pure data transforms — no shared state.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// One-shot poll of `nvidia-smi pmon -c 1`, returning a map PID → SM utilization
/// (0..100). On a 4-GPU host with one PID using two GPUs at 50 % each, that
/// PID appears twice and the caller sums the values. Empty map when nvidia-smi
/// is missing, fails, or the line format is unexpected — caller falls back to
/// 0 % GPU usage gracefully.
pub(super) fn sample_gpu_per_pid() -> HashMap<u32, f32> {
    let output = std::process::Command::new("nvidia-smi")
        .args(["pmon", "-c", "1", "-s", "u"])
        .output();
    let mut out: HashMap<u32, f32> = HashMap::new();
    let Ok(o) = output else { return out };
    if !o.status.success() {
        return out;
    }
    let stdout = String::from_utf8_lossy(&o.stdout);
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Expected columns: gpu pid type sm mem enc dec command
        // Some drivers emit "-" for sm/mem when the process is short-lived.
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 4 {
            continue;
        }
        let Ok(pid) = parts[1].parse::<u32>() else {
            continue;
        };
        let sm: f32 = parts[3].parse().unwrap_or(0.0);
        *out.entry(pid).or_insert(0.0) += sm;
    }
    out
}

/// Read the cgroup-accounted RSS of a sandbox task in megabytes. Reads
/// `/proc/<root_pid>/cgroup` to find which sub-cgroup the bwrap parent
/// lives in (`/sys/fs/cgroup/partagpu/task-<uuid>/`), then reads
/// `memory.current` from that cgroup.
///
/// Why : summing `process.memory()` (RSS) across the descendant tree
/// double-counts shared memory pages — a PyTorch worker tree with
/// 3 processes each loading libtorch reports ~3× the real footprint.
/// `memory.current` is the kernel's authoritative tally for that cgroup,
/// computed without double-counting (PSS-style accounting at page level).
///
/// Returns `None` if the process isn't in a partagpu sub-cgroup (fallback
/// path with no cgroup, or cgroup v1 layout — the caller falls back to
/// the RSS sum which is at least a useful upper bound).
pub(super) fn cgroup_memory_mb(root_pid: u32) -> Option<u64> {
    let cgroup_file = format!("/proc/{root_pid}/cgroup");
    let content = std::fs::read_to_string(&cgroup_file).ok()?;
    // cgroup v2 unified hierarchy : one line per process, format `0::/<path>`.
    // We accept any line that mentions the partagpu task hierarchy, so this
    // also works if cgroup v1 controllers happen to be mounted alongside.
    let rel = content.lines().find_map(|line| {
        let (_, path) = line.split_once("::")?;
        let path = path.trim();
        if path.contains("/partagpu/task-") {
            Some(path.to_string())
        } else {
            None
        }
    })?;
    let abs = PathBuf::from("/sys/fs/cgroup").join(rel.trim_start_matches('/'));
    let mem_bytes: u64 = std::fs::read_to_string(abs.join("memory.current"))
        .ok()?
        .trim()
        .parse()
        .ok()?;
    Some(mem_bytes / (1024 * 1024))
}

/// Collect the BFS process tree rooted at `root`, using the parent-child
/// links exposed by sysinfo. Used to aggregate CPU/RAM of a sandbox task
/// (the bwrap parent + python + any further children).
pub(super) fn collect_descendants(
    sys: &sysinfo::System,
    root: sysinfo::Pid,
) -> HashSet<sysinfo::Pid> {
    let mut tree: HashSet<sysinfo::Pid> = HashSet::new();
    tree.insert(root);
    let mut changed = true;
    while changed {
        changed = false;
        for (pid, process) in sys.processes() {
            if tree.contains(pid) {
                continue;
            }
            if let Some(parent) = process.parent() {
                if tree.contains(&parent) {
                    tree.insert(*pid);
                    changed = true;
                }
            }
        }
    }
    tree
}
