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

/// Read the cgroup-accounted RAM of a sandbox task in megabytes. Reads
/// `/proc/<root_pid>/cgroup` to find which sub-cgroup the bwrap parent
/// lives in (`/sys/fs/cgroup/partagpu/task-<uuid>/`), then parses
/// `memory.stat` from that cgroup and sums `anon + kernel_stack + pagetables
/// + slab + sock` — i.e. memory that is actually allocated to the task and
/// not reclaimable file cache.
///
/// Why not `memory.current` : that field includes file-backed page cache.
/// A task that mmap'd a 10 GiB dataset shows 10 GiB in `memory.current`,
/// but those pages are evictable and the host kernel reports them as
/// "available", so `sysinfo::used_memory()` excludes them. Comparing the
/// two leads to the per-user segments dwarfing the system "used" total
/// (segments based on `memory.current`, total based on sysinfo) — the
/// gauge ends up showing one peer "using" more than the whole machine.
/// Summing only the unreclaimable fields gives a number that is directly
/// comparable to sysinfo's view, so the segments fit inside the total.
///
/// Why summing `process.memory()` (RSS) across descendants is also wrong :
/// it double-counts shared library pages — a PyTorch worker tree with
/// 3 processes each loading libtorch would report ~3× the real footprint.
/// The cgroup's own `anon` field is PSS-accounted at the page level.
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
    let stat = std::fs::read_to_string(abs.join("memory.stat")).ok()?;
    // Sum only the fields that represent unreclaimable memory truly held
    // by the task. Skip `file` (page cache, evictable) and `shmem` (already
    // counted as anon in cgroup v2). Missing keys silently contribute 0.
    const COUNTED: &[&str] = &["anon", "kernel_stack", "pagetables", "slab", "sock"];
    let mut total_bytes: u64 = 0;
    for line in stat.lines() {
        let mut parts = line.split_whitespace();
        let Some(key) = parts.next() else { continue };
        if !COUNTED.contains(&key) {
            continue;
        }
        if let Some(val) = parts.next().and_then(|s| s.parse::<u64>().ok()) {
            total_bytes = total_bytes.saturating_add(val);
        }
    }
    Some(total_bytes / (1024 * 1024))
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
