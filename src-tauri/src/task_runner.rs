use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use crate::sandbox::{OutputSink, Sandbox, SandboxOptions, SandboxResult};

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
    sandbox: Sandbox,
}

impl IncomingTasks {
    pub fn new(sandbox: Sandbox) -> Self {
        Self {
            tasks: Arc::new(Mutex::new(HashMap::new())),
            pids: Arc::new(Mutex::new(HashMap::new())),
            sinks: Arc::new(Mutex::new(HashMap::new())),
            sandbox,
        }
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
        let mut map = self.tasks.lock().unwrap();
        map.insert(task.id.clone(), task);
    }

    pub fn update_status(&self, id: &str, status: TaskStatus) {
        let mut map = self.tasks.lock().unwrap();
        if let Some(task) = map.get_mut(id) {
            task.status = status;
        }
    }

    pub fn remove(&self, id: &str) {
        self.tasks.lock().unwrap().remove(id);
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

        {
            let mut map = tasks.lock().unwrap();
            if let Some(task) = map.get_mut(&id) {
                task.status = TaskStatus::Running;
            }
        }

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
}

#[derive(Clone, Debug)]
pub struct RemoteRef {
    pub peer_ip: String,
    pub remote_task_id: String,
}

impl OutgoingTasks {
    pub fn new() -> Self {
        Self {
            tasks: Arc::new(Mutex::new(HashMap::new())),
            remote_refs: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn list(&self) -> Vec<Task> {
        self.tasks.lock().unwrap().values().cloned().collect()
    }

    pub fn get(&self, id: &str) -> Option<Task> {
        self.tasks.lock().unwrap().get(id).cloned()
    }

    pub fn add(&self, task: Task) {
        let mut map = self.tasks.lock().unwrap();
        map.insert(task.id.clone(), task);
    }

    /// Replace the full Task record (used when polling refreshes from the peer).
    pub fn replace(&self, task: Task) {
        let mut map = self.tasks.lock().unwrap();
        map.insert(task.id.clone(), task);
    }

    pub fn update_progress(&self, id: &str, progress: f32, status: TaskStatus) {
        let mut map = self.tasks.lock().unwrap();
        if let Some(task) = map.get_mut(id) {
            task.progress = progress;
            task.status = status;
        }
    }

    pub fn set_failed(&self, id: &str, error: &str) {
        let mut map = self.tasks.lock().unwrap();
        if let Some(task) = map.get_mut(id) {
            task.status = TaskStatus::Failed;
            task.error_output = error.to_string();
        }
    }

    pub fn set_cancelled(&self, id: &str) {
        let mut map = self.tasks.lock().unwrap();
        if let Some(task) = map.get_mut(id) {
            task.status = TaskStatus::Cancelled;
        }
    }

    /// Mirror partial stdout/stderr from a peer into the local OutgoingTask.
    /// Called by the dispatch poll loop so the UI can show live output.
    pub fn update_outputs(&self, id: &str, stdout: &str, stderr: &str) {
        let mut map = self.tasks.lock().unwrap();
        if let Some(task) = map.get_mut(id) {
            task.output = stdout.to_string();
            task.error_output = stderr.to_string();
        }
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
    }
}
