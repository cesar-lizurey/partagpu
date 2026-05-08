//! Tasks I run on behalf of others (incoming dispatches from peers).
//!
//! [`IncomingTasks`] owns the bookkeeping for tasks being executed locally
//! in the sandbox : the live PID map (for cancellation), the per-task
//! `OutputSink` for live stdout/stderr streaming, the start-time +
//! timeout maps used to compute `progress = elapsed / timeout`, the
//! `pending` FIFO that holds tasks parked above the concurrency cap,
//! and the optional Tauri `AppHandle` for pushing
//! `incoming-tasks-changed` events to the frontend.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use super::monitor::{cgroup_memory_mb, collect_descendants, sample_gpu_per_pid};
use super::{
    config_dir, new_task, save_atomic, task_for_persist, Task, TaskStatus, PERSIST_FLUSH_INTERVAL,
};
use crate::sandbox::{OutputSink, Sandbox, SandboxOptions, SandboxResult};

/// Default cap on tasks running concurrently on this machine. Anything beyond
/// this stays Queued and starts when a slot frees. Picked so a peer can't
/// flood us with 100 simultaneous bwraps; tweakable later via the UI.
const DEFAULT_MAX_CONCURRENT: usize = 4;

/// Tasks I run on behalf of others (incoming).
#[derive(Clone)]
pub struct IncomingTasks {
    tasks: Arc<Mutex<HashMap<String, Task>>>,
    /// PID of the bwrap process per running task. Populated when the task
    /// transitions to Running, removed when the wait loop returns.
    pids: Arc<Mutex<HashMap<String, u32>>>,
    /// Live stdout/stderr sinks while a task is running. Populated by
    /// spawn_execution, removed when the task reaches a terminal state.
    /// Lets `get(id)` return a fresh snapshot of partial output.
    sinks: Arc<Mutex<HashMap<String, OutputSink>>>,
    /// When each running task entered the Running state. Used by the monitor
    /// thread to compute progress as elapsed/timeout.
    task_starts: Arc<Mutex<HashMap<String, Instant>>>,
    /// Timeout in seconds per task, for the same progress computation.
    task_timeouts: Arc<Mutex<HashMap<String, u64>>>,
    sandbox: Sandbox,
    /// AppHandle used to push "incoming-tasks-changed" Tauri events whenever
    /// the task list mutates. Set once after Tauri starts via `set_emitter`.
    emitter: Arc<Mutex<Option<tauri::AppHandle>>>,
    /// Tasks waiting for a free slot. Each entry carries the runtime params
    /// the executor will need when it becomes runnable.
    pending: Arc<Mutex<VecDeque<PendingTask>>>,
    /// Max concurrent Running tasks. Anything more goes to `pending`.
    max_concurrent: Arc<Mutex<usize>>,
}

/// Backing data for a Queued task waiting for a free execution slot. We hold
/// the SandboxOptions / timeout off-band because they're not part of `Task`.
struct PendingTask {
    id: String,
    timeout_secs: u64,
    options: SandboxOptions,
}

impl IncomingTasks {
    pub fn new(sandbox: Sandbox) -> Self {
        let this = Self {
            tasks: Arc::new(Mutex::new(HashMap::new())),
            pids: Arc::new(Mutex::new(HashMap::new())),
            sinks: Arc::new(Mutex::new(HashMap::new())),
            task_starts: Arc::new(Mutex::new(HashMap::new())),
            task_timeouts: Arc::new(Mutex::new(HashMap::new())),
            sandbox,
            emitter: Arc::new(Mutex::new(None)),
            pending: Arc::new(Mutex::new(VecDeque::new())),
            max_concurrent: Arc::new(Mutex::new(DEFAULT_MAX_CONCURRENT)),
        };
        this.load_from_disk();
        this.spawn_monitor();
        this.spawn_persistence();
        this
    }

    /// Read the current concurrency cap. Used by the UI to display it.
    pub fn max_concurrent(&self) -> usize {
        *self.max_concurrent.lock().unwrap()
    }

    /// Set the max number of tasks that may run at once. Lowering it doesn't
    /// kill in-flight tasks, only delays new arrivals. Raising it pulls from
    /// the pending queue immediately.
    pub fn set_max_concurrent(&self, n: usize) {
        let new = n.max(1);
        *self.max_concurrent.lock().unwrap() = new;
        // Pull as many as we can now that the cap rose.
        while self.try_start_pending() {}
    }

    /// Count Running tasks. Pending ones are still Queued.
    fn running_count(&self) -> usize {
        self.tasks
            .lock()
            .unwrap()
            .values()
            .filter(|t| t.status == TaskStatus::Running)
            .count()
    }

    /// If there's a free slot AND a queued task, dequeue + spawn it.
    /// Returns true if it actually started something.
    fn try_start_pending(&self) -> bool {
        if self.running_count() >= self.max_concurrent() {
            return false;
        }
        let next = self.pending.lock().unwrap().pop_front();
        let Some(p) = next else { return false };
        // The task may have been cancelled while waiting in the queue —
        // skip it and try the next one.
        let still_queued = matches!(
            self.tasks.lock().unwrap().get(&p.id).map(|t| t.status),
            Some(TaskStatus::Queued)
        );
        if !still_queued {
            return self.try_start_pending();
        }
        self.spawn_execution(&p.id, p.timeout_secs, p.options);
        true
    }

    /// Plug in the AppHandle so subsequent mutations push events to the UI.
    /// Called once from `lib.rs::run` after the Tauri builder hands us a handle.
    pub fn set_emitter(&self, app: tauri::AppHandle) {
        *self.emitter.lock().unwrap() = Some(app);
    }

    /// Emit the current task list to the frontend. Must be called WITHOUT
    /// holding the `tasks` lock (it locks internally).
    fn notify(&self) {
        let app = match self.emitter.lock().unwrap().clone() {
            Some(a) => a,
            None => return,
        };
        use tauri::Emitter;
        let payload = self.list();
        let _ = app.emit("incoming-tasks-changed", &payload);
    }

    fn save_path() -> PathBuf {
        config_dir().join("incoming-tasks.json")
    }

    fn load_from_disk(&self) {
        let path = Self::save_path();
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => return, // first run, no file yet
        };
        let loaded: HashMap<String, Task> = match serde_json::from_str(&content) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("incoming-tasks.json corrompu, ignoré : {e}");
                return;
            }
        };
        let mut map = self.tasks.lock().unwrap();
        for (id, mut task) in loaded {
            // Tasks that were Running/Queued at shutdown are dead now.
            if matches!(task.status, TaskStatus::Running | TaskStatus::Queued) {
                task.status = TaskStatus::Cancelled;
                task.error_output = if task.error_output.is_empty() {
                    "Tâche interrompue par redémarrage de l'application.".to_string()
                } else {
                    task.error_output
                };
            }
            map.insert(id, task);
        }
    }

    fn spawn_persistence(&self) {
        let tasks = self.tasks.clone();
        std::thread::spawn(move || {
            let path = Self::save_path();
            loop {
                std::thread::sleep(PERSIST_FLUSH_INTERVAL);
                let snapshot: HashMap<String, Task> = tasks
                    .lock()
                    .unwrap()
                    .iter()
                    .map(|(k, v)| (k.clone(), task_for_persist(v)))
                    .collect();
                if let Err(e) = save_atomic(&path, &snapshot) {
                    eprintln!("persist incoming-tasks: {e}");
                }
            }
        });
    }

    /// Background thread that walks running tasks every second, updating
    /// progress (elapsed/timeout) + CPU/RAM usage per task by aggregating
    /// the bwrap process tree via sysinfo. Lifelong (runs as long as the
    /// IncomingTasks Arcs are alive).
    fn spawn_monitor(&self) {
        let tasks = self.tasks.clone();
        let pids = self.pids.clone();
        let starts = self.task_starts.clone();
        let timeouts = self.task_timeouts.clone();
        let this = self.clone();

        std::thread::spawn(move || {
            use sysinfo::{Pid, ProcessesToUpdate, System};
            let mut sys = System::new();
            // Two refreshes back-to-back so cpu_usage has a baseline.
            sys.refresh_processes(ProcessesToUpdate::All, true);

            loop {
                std::thread::sleep(std::time::Duration::from_secs(1));
                sys.refresh_processes(ProcessesToUpdate::All, true);

                let snapshot_pids: HashMap<String, u32> = pids.lock().unwrap().clone();
                if snapshot_pids.is_empty() {
                    continue;
                }
                let mut changed = false;
                let snapshot_starts: HashMap<String, Instant> = starts.lock().unwrap().clone();
                let snapshot_timeouts: HashMap<String, u64> = timeouts.lock().unwrap().clone();

                // One-shot poll of nvidia-smi pmon to learn per-PID GPU SM%.
                // Empty map when the host has no GPU or pmon failed; we just
                // leave gpu_usage at 0 in that case.
                let gpu_per_pid = sample_gpu_per_pid();

                for (task_id, root_pid) in snapshot_pids {
                    // Walk the process tree rooted at the bwrap PID, summing
                    // CPU%. The sandbox creates child processes (python, etc.)
                    // that account for most of the work.
                    let tree = collect_descendants(&sys, Pid::from_u32(root_pid));
                    let mut cpu_sum: f32 = 0.0;
                    let mut rss_sum_bytes: u64 = 0;
                    let mut gpu_sum: f32 = 0.0;
                    for pid in &tree {
                        if let Some(p) = sys.process(*pid) {
                            cpu_sum += p.cpu_usage();
                            rss_sum_bytes += p.memory();
                        }
                        if let Some(util) = gpu_per_pid.get(&(pid.as_u32())) {
                            gpu_sum += *util;
                        }
                    }
                    // RAM : prefer the kernel's per-cgroup tally
                    // (`memory.current` of the task's sub-cgroup) — it
                    // accounts shared pages once, which the sum of
                    // per-process RSS does not. Fall back to the RSS sum
                    // when the process isn't in a partagpu sub-cgroup
                    // (e.g. cgroup creation failed at sandbox start).
                    let ram_mb =
                        cgroup_memory_mb(root_pid).unwrap_or(rss_sum_bytes / (1024 * 1024));
                    let gpu_pct = gpu_sum.min(100.0 * tree.len().max(1) as f32);

                    let progress = match (
                        snapshot_starts.get(&task_id),
                        snapshot_timeouts.get(&task_id),
                    ) {
                        (Some(start), Some(timeout)) if *timeout > 0 => {
                            let elapsed = start.elapsed().as_secs_f32();
                            ((elapsed / *timeout as f32) * 100.0).min(99.0)
                        }
                        _ => 50.0,
                    };

                    if let Some(task) = tasks.lock().unwrap().get_mut(&task_id) {
                        if task.status == TaskStatus::Running {
                            task.progress = progress;
                            task.cpu_usage = cpu_sum;
                            task.ram_usage_mb = ram_mb;
                            task.gpu_usage = gpu_pct;
                            changed = true;
                        }
                    }
                }

                if changed {
                    this.notify();
                }
            }
        });
    }

    pub fn list(&self) -> Vec<Task> {
        let sinks = self.sinks.lock().unwrap();
        self.tasks
            .lock()
            .unwrap()
            .values()
            .map(|t| {
                let mut task = t.clone();
                if let Some(sink) = sinks.get(&task.id) {
                    let (out, err) = sink.snapshot();
                    task.output = out;
                    task.error_output = err;
                }
                task
            })
            .collect()
    }

    /// Return a snapshot of the task. If the task is still running, the
    /// `output` / `error_output` fields are taken from the live OutputSink
    /// (latest partial bytes); otherwise from the final state stored at exit.
    pub fn get(&self, id: &str) -> Option<Task> {
        let mut task = self.tasks.lock().unwrap().get(id).cloned()?;
        if let Some(sink) = self.sinks.lock().unwrap().get(id) {
            let (out, err) = sink.snapshot();
            task.output = out;
            task.error_output = err;
        }
        Some(task)
    }

    pub fn add(&self, task: Task) {
        {
            let mut map = self.tasks.lock().unwrap();
            map.insert(task.id.clone(), task);
        }
        self.notify();
    }

    pub fn update_status(&self, id: &str, status: TaskStatus) {
        {
            let mut map = self.tasks.lock().unwrap();
            if let Some(task) = map.get_mut(id) {
                task.status = status;
            }
        }
        self.notify();
    }

    pub fn remove(&self, id: &str) {
        self.tasks.lock().unwrap().remove(id);
        self.notify();
    }

    /// Create a Task, add it to the queue, and execute it asynchronously.
    /// Returns the queued Task immediately. Use `get(id)` to poll for completion.
    /// If the concurrency cap is reached, the task stays Queued and will be
    /// started by `try_start_pending` when a slot frees.
    pub fn create_and_run(
        &self,
        args: Vec<String>,
        source_machine: String,
        source_user: String,
        target_machine: String,
        timeout_secs: u64,
        options: SandboxOptions,
    ) -> Result<Task, String> {
        if args.is_empty() {
            return Err("La commande ne peut pas être vide.".into());
        }
        let mut task = new_task(args, source_machine, source_user, target_machine);
        task.network_enabled = options.network_enabled;
        let task_id = task.id.clone();
        let task_clone = task.clone();
        self.add(task);

        if self.running_count() < self.max_concurrent() {
            self.spawn_execution(&task_id, timeout_secs, options);
        } else {
            // Park it: stays in `tasks` as Queued, gets executed when a slot
            // frees up. The persistence loop will save the Queued status.
            self.pending.lock().unwrap().push_back(PendingTask {
                id: task_id,
                timeout_secs,
                options,
            });
            self.notify();
        }
        Ok(task_clone)
    }

    /// Execute a task inside the sandbox. Runs in a background thread.
    /// Updates the task status and output when done.
    fn spawn_execution(&self, task_id: &str, timeout_secs: u64, options: SandboxOptions) {
        let tasks = self.tasks.clone();
        let pids = self.pids.clone();
        let sinks = self.sinks.clone();
        let sandbox = self.sandbox.clone();
        let id = task_id.to_string();
        let this = self.clone();

        let args = {
            let map = tasks.lock().unwrap();
            match map.get(&id) {
                Some(task) => task.args.clone(),
                None => return,
            }
        };

        // Register a live output sink BEFORE starting the sandbox so polling
        // clients see partial output as soon as the first chunk arrives.
        let sink = OutputSink::new();
        sinks.lock().unwrap().insert(id.clone(), sink.clone());

        // Record start time + timeout so the monitor thread can compute
        // progress as elapsed/timeout.
        self.task_starts
            .lock()
            .unwrap()
            .insert(id.clone(), Instant::now());
        self.task_timeouts
            .lock()
            .unwrap()
            .insert(id.clone(), timeout_secs);
        let task_starts = self.task_starts.clone();
        let task_timeouts = self.task_timeouts.clone();

        {
            let mut map = tasks.lock().unwrap();
            if let Some(task) = map.get_mut(&id) {
                task.status = TaskStatus::Running;
            }
        }
        this.notify();

        std::thread::spawn(move || {
            let pids_for_callback = pids.clone();
            let id_for_callback = id.clone();
            let result = sandbox.execute_with_callbacks_and_sink(
                &args,
                timeout_secs,
                &options,
                move |pid| {
                    pids_for_callback
                        .lock()
                        .unwrap()
                        .insert(id_for_callback, pid);
                },
                Some(&sink),
            );

            // Always remove the PID entry when the wait loop exits (process is dead).
            pids.lock().unwrap().remove(&id);

            {
                let mut map = tasks.lock().unwrap();
                if let Some(task) = map.get_mut(&id) {
                    let already_cancelled = task.status == TaskStatus::Cancelled;
                    match result {
                        Ok(SandboxResult {
                            exit_code,
                            stdout,
                            stderr,
                            artifacts,
                        }) => {
                            task.output = stdout;
                            task.error_output = stderr;
                            task.exit_code = Some(exit_code);
                            task.artifacts = artifacts;
                            if !already_cancelled {
                                task.progress = 100.0;
                                task.status = if exit_code == 0 {
                                    TaskStatus::Completed
                                } else {
                                    TaskStatus::Failed
                                };
                            }
                            // For cancelled tasks, keep the progress at whatever
                            // value was set when cancellation arrived.
                        }
                        Err(e) => {
                            if !already_cancelled {
                                task.error_output = e;
                                task.status = TaskStatus::Failed;
                            }
                        }
                    }
                }
            }

            // Drop the live sink — subsequent get(id) calls will return the
            // final output captured in the Task itself, not the sink mirror.
            sinks.lock().unwrap().remove(&id);
            task_starts.lock().unwrap().remove(&id);
            task_timeouts.lock().unwrap().remove(&id);
            this.notify();

            // A slot just freed — pull the next Queued task from the pending
            // queue if any. Loops if a head entry was already cancelled.
            this.try_start_pending();
        });
    }

    /// Mark a task as Cancelled and SIGTERM its bwrap process (then SIGKILL after 2 s).
    /// Returns Err if the task doesn't exist or has already finished.
    pub fn cancel(&self, task_id: &str) -> Result<(), String> {
        // Mark Cancelled first so the wait-loop completion handler doesn't
        // overwrite the status with Failed.
        {
            let mut map = self.tasks.lock().unwrap();
            match map.get_mut(task_id) {
                None => return Err("Tâche introuvable.".into()),
                Some(task) => match task.status {
                    TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Cancelled => {
                        return Err("Tâche déjà terminée.".into());
                    }
                    _ => task.status = TaskStatus::Cancelled,
                },
            }
        }
        self.notify();

        let pid = self.pids.lock().unwrap().get(task_id).copied();
        if let Some(pid) = pid {
            let pid_str = pid.to_string();
            // Best effort: TERM, then KILL after 2 s if still alive.
            let _ = std::process::Command::new("kill")
                .args(["-TERM", &pid_str])
                .status();
            let pids = self.pids.clone();
            let id = task_id.to_string();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(2));
                if pids.lock().unwrap().contains_key(&id) {
                    let _ = std::process::Command::new("kill")
                        .args(["-KILL", &pid_str])
                        .status();
                }
            });
        }
        Ok(())
    }

    pub fn get_sandbox(&self) -> &Sandbox {
        &self.sandbox
    }
}
