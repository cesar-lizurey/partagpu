use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime};

use crate::sandbox::{OutputSink, Sandbox, SandboxOptions, SandboxResult};

/// Per-task output cap when persisting to disk. Avoids saving 1 MB stdout
/// for every task across restarts (x100 tasks would be 100 MB on disk).
const PERSIST_OUTPUT_CAP: usize = 50 * 1024;
const PERSIST_FLUSH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

fn config_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("partagpu")
}

fn save_atomic<T: Serialize>(path: &std::path::Path, value: &T) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    let json = serde_json::to_string_pretty(value)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    std::fs::write(&tmp, json)?;
    std::fs::rename(tmp, path)
}

/// Truncate output fields before persisting so a chatty task doesn't blow
/// up the saved file. Never modifies the in-memory Task — only the saved
/// copy.
fn task_for_persist(task: &Task) -> Task {
    let mut t = task.clone();
    if t.output.len() > PERSIST_OUTPUT_CAP {
        t.output.truncate(PERSIST_OUTPUT_CAP);
        t.output.push_str("\n[…tronqué pour persistance]");
    }
    if t.error_output.len() > PERSIST_OUTPUT_CAP {
        t.error_output.truncate(PERSIST_OUTPUT_CAP);
        t.error_output.push_str("\n[…tronqué pour persistance]");
    }
    t
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub command: String,
    pub args: Vec<String>,
    pub source_machine: String,
    pub source_user: String,
    pub target_machine: String,
    pub status: TaskStatus,
    pub progress: f32,
    pub cpu_usage: f32,
    pub ram_usage_mb: u64,
    pub gpu_usage: f32,
    pub output: String,
    pub error_output: String,
    pub exit_code: Option<i32>,
    pub created_at: u64,
    /// Whether the sandbox was launched with host network access (DDP rendezvous).
    /// Surfaced to the UI as a "network" indicator.
    #[serde(default)]
    pub network_enabled: bool,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Build a fresh Task with default lifecycle fields.
pub fn new_task(
    args: Vec<String>,
    source_machine: String,
    source_user: String,
    target_machine: String,
) -> Task {
    let cmd_str = args.join(" ");
    Task {
        id: uuid::Uuid::new_v4().to_string(),
        command: cmd_str,
        args,
        source_machine,
        source_user,
        target_machine,
        status: TaskStatus::Queued,
        progress: 0.0,
        cpu_usage: 0.0,
        ram_usage_mb: 0,
        gpu_usage: 0.0,
        output: String::new(),
        error_output: String::new(),
        exit_code: None,
        created_at: now_secs(),
        network_enabled: false,
    }
}

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
        };
        this.load_from_disk();
        this.spawn_monitor();
        this.spawn_persistence();
        this
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

                let snapshot_pids: HashMap<String, u32> =
                    pids.lock().unwrap().clone();
                if snapshot_pids.is_empty() {
                    continue;
                }
                let mut changed = false;
                let snapshot_starts: HashMap<String, Instant> =
                    starts.lock().unwrap().clone();
                let snapshot_timeouts: HashMap<String, u64> =
                    timeouts.lock().unwrap().clone();

                for (task_id, root_pid) in snapshot_pids {
                    // Walk the process tree rooted at the bwrap PID, summing
                    // CPU% and RAM. The sandbox creates child processes
                    // (python, etc.) that account for most of the work.
                    let tree = collect_descendants(&sys, Pid::from_u32(root_pid));
                    let mut cpu_sum: f32 = 0.0;
                    let mut ram_sum: u64 = 0;
                    for pid in &tree {
                        if let Some(p) = sys.process(*pid) {
                            cpu_sum += p.cpu_usage();
                            ram_sum += p.memory();
                        }
                    }
                    let ram_mb = ram_sum / (1024 * 1024);

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
        self.spawn_execution(&task_id, timeout_secs, options);
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
                    pids_for_callback.lock().unwrap().insert(id_for_callback, pid);
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
                        Ok(SandboxResult { exit_code, stdout, stderr }) => {
                            task.output = stdout;
                            task.error_output = stderr;
                            task.exit_code = Some(exit_code);
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

/// Tasks I submitted to other machines (outgoing).
#[derive(Clone)]
pub struct OutgoingTasks {
    tasks: Arc<Mutex<HashMap<String, Task>>>,
    /// For each local outgoing task id, the (peer_ip, remote_task_id) on the
    /// peer that runs it. Used to propagate cancellation via DELETE.
    remote_refs: Arc<Mutex<HashMap<String, RemoteRef>>>,
    /// AppHandle used to push "outgoing-tasks-changed" Tauri events whenever
    /// the task list mutates. Set once after Tauri starts via `set_emitter`.
    emitter: Arc<Mutex<Option<tauri::AppHandle>>>,
}

#[derive(Clone, Debug)]
pub struct RemoteRef {
    pub peer_ip: String,
    pub remote_task_id: String,
}

#[derive(Serialize, Deserialize, Default)]
struct OutgoingPersisted {
    #[serde(default)]
    tasks: HashMap<String, Task>,
    #[serde(default)]
    remote_refs: HashMap<String, RemoteRef>,
}

// Make RemoteRef serializable for persistence
impl Serialize for RemoteRef {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut st = s.serialize_struct("RemoteRef", 2)?;
        st.serialize_field("peer_ip", &self.peer_ip)?;
        st.serialize_field("remote_task_id", &self.remote_task_id)?;
        st.end()
    }
}
impl<'de> Deserialize<'de> for RemoteRef {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct R {
            peer_ip: String,
            remote_task_id: String,
        }
        let r = R::deserialize(d)?;
        Ok(RemoteRef {
            peer_ip: r.peer_ip,
            remote_task_id: r.remote_task_id,
        })
    }
}

impl OutgoingTasks {
    pub fn new() -> Self {
        let this = Self {
            tasks: Arc::new(Mutex::new(HashMap::new())),
            remote_refs: Arc::new(Mutex::new(HashMap::new())),
            emitter: Arc::new(Mutex::new(None)),
        };
        this.load_from_disk();
        this.spawn_persistence();
        this
    }

    /// Plug in the AppHandle so subsequent mutations push events to the UI.
    pub fn set_emitter(&self, app: tauri::AppHandle) {
        *self.emitter.lock().unwrap() = Some(app);
    }

    /// Emit the current task list to the frontend. Must be called WITHOUT
    /// holding the `tasks` lock.
    fn notify(&self) {
        let app = match self.emitter.lock().unwrap().clone() {
            Some(a) => a,
            None => return,
        };
        use tauri::Emitter;
        let payload = self.list();
        let _ = app.emit("outgoing-tasks-changed", &payload);
    }

    fn save_path() -> PathBuf {
        config_dir().join("outgoing-tasks.json")
    }

    fn load_from_disk(&self) {
        let path = Self::save_path();
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => return,
        };
        let loaded: OutgoingPersisted = match serde_json::from_str(&content) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("outgoing-tasks.json corrompu, ignoré : {e}");
                return;
            }
        };
        let mut tasks = self.tasks.lock().unwrap();
        for (id, mut task) in loaded.tasks {
            if matches!(task.status, TaskStatus::Running | TaskStatus::Queued) {
                task.status = TaskStatus::Cancelled;
                task.error_output = if task.error_output.is_empty() {
                    "Dispatch interrompu par redémarrage de l'application.".to_string()
                } else {
                    task.error_output
                };
            }
            tasks.insert(id, task);
        }
        // remote_refs are stale once we restart (the peer's task is gone),
        // but loading them is harmless. They get cleared as the cancelled
        // tasks above don't reach the cleanup paths. Skip loading.
    }

    fn spawn_persistence(&self) {
        let tasks = self.tasks.clone();
        let remote_refs = self.remote_refs.clone();
        std::thread::spawn(move || {
            let path = Self::save_path();
            loop {
                std::thread::sleep(PERSIST_FLUSH_INTERVAL);
                let payload = OutgoingPersisted {
                    tasks: tasks
                        .lock()
                        .unwrap()
                        .iter()
                        .map(|(k, v)| (k.clone(), task_for_persist(v)))
                        .collect(),
                    remote_refs: remote_refs.lock().unwrap().clone(),
                };
                if let Err(e) = save_atomic(&path, &payload) {
                    eprintln!("persist outgoing-tasks: {e}");
                }
            }
        });
    }

    pub fn list(&self) -> Vec<Task> {
        self.tasks.lock().unwrap().values().cloned().collect()
    }

    pub fn get(&self, id: &str) -> Option<Task> {
        self.tasks.lock().unwrap().get(id).cloned()
    }

    pub fn add(&self, task: Task) {
        {
            let mut map = self.tasks.lock().unwrap();
            map.insert(task.id.clone(), task);
        }
        self.notify();
    }

    /// Replace the full Task record (used when polling refreshes from the peer).
    pub fn replace(&self, task: Task) {
        {
            let mut map = self.tasks.lock().unwrap();
            map.insert(task.id.clone(), task);
        }
        self.notify();
    }

    pub fn update_progress(&self, id: &str, progress: f32, status: TaskStatus) {
        {
            let mut map = self.tasks.lock().unwrap();
            if let Some(task) = map.get_mut(id) {
                task.progress = progress;
                task.status = status;
            }
        }
        self.notify();
    }

    pub fn set_failed(&self, id: &str, error: &str) {
        {
            let mut map = self.tasks.lock().unwrap();
            if let Some(task) = map.get_mut(id) {
                task.status = TaskStatus::Failed;
                task.error_output = error.to_string();
            }
        }
        self.notify();
    }

    pub fn set_cancelled(&self, id: &str) {
        {
            let mut map = self.tasks.lock().unwrap();
            if let Some(task) = map.get_mut(id) {
                task.status = TaskStatus::Cancelled;
            }
        }
        self.notify();
    }

    /// Mirror partial stdout/stderr from a peer into the local OutgoingTask.
    /// Called by the dispatch poll loop so the UI can show live output.
    pub fn update_outputs(&self, id: &str, stdout: &str, stderr: &str) {
        {
            let mut map = self.tasks.lock().unwrap();
            if let Some(task) = map.get_mut(id) {
                task.output = stdout.to_string();
                task.error_output = stderr.to_string();
            }
        }
        self.notify();
    }

    /// Mirror live metrics (output + progress + CPU/RAM/GPU) from a still
    /// -running peer task into the local OutgoingTask. Status field is left
    /// alone — the caller manages the lifecycle.
    pub fn mirror_running(&self, id: &str, peer: &Task) {
        {
            let mut map = self.tasks.lock().unwrap();
            if let Some(task) = map.get_mut(id) {
                task.output = peer.output.clone();
                task.error_output = peer.error_output.clone();
                task.progress = peer.progress;
                task.cpu_usage = peer.cpu_usage;
                task.ram_usage_mb = peer.ram_usage_mb;
                task.gpu_usage = peer.gpu_usage;
            }
        }
        self.notify();
    }

    pub fn set_remote_ref(&self, local_id: &str, peer_ip: &str, remote_task_id: &str) {
        self.remote_refs.lock().unwrap().insert(
            local_id.to_string(),
            RemoteRef {
                peer_ip: peer_ip.to_string(),
                remote_task_id: remote_task_id.to_string(),
            },
        );
    }

    pub fn get_remote_ref(&self, local_id: &str) -> Option<RemoteRef> {
        self.remote_refs.lock().unwrap().get(local_id).cloned()
    }

    pub fn clear_remote_ref(&self, local_id: &str) {
        self.remote_refs.lock().unwrap().remove(local_id);
    }

    pub fn remove(&self, id: &str) {
        self.tasks.lock().unwrap().remove(id);
        self.remote_refs.lock().unwrap().remove(id);
        self.notify();
    }
}

/// Collect the BFS process tree rooted at `root`, using the parent-child
/// links exposed by sysinfo. Used to aggregate CPU/RAM of a sandbox task
/// (the bwrap parent + python + any further children).
fn collect_descendants(sys: &sysinfo::System, root: sysinfo::Pid) -> HashSet<sysinfo::Pid> {
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
