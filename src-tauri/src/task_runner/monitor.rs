//! Process-tree resource sampling helpers used by the per-task monitor
//! thread. Both functions are pure data transforms — no shared state.

use std::collections::{HashMap, HashSet};

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
