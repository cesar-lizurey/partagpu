use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use crate::sandbox::{Sandbox, SandboxOptions, SandboxResult};

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
    sandbox: Sandbox,
}

impl IncomingTasks {
    pub fn new(sandbox: Sandbox) -> Self {
        Self {
            tasks: Arc::new(Mutex::new(HashMap::new())),
            sandbox,
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
        let sandbox = self.sandbox.clone();
        let id = task_id.to_string();

        let args = {
            let map = tasks.lock().unwrap();
            match map.get(&id) {
                Some(task) => task.args.clone(),
                None => return,
            }
        };

        {
            let mut map = tasks.lock().unwrap();
            if let Some(task) = map.get_mut(&id) {
                task.status = TaskStatus::Running;
            }
        }

        std::thread::spawn(move || {
            let result = sandbox.execute_with_options(&args, timeout_secs, &options);
            let mut map = tasks.lock().unwrap();
            if let Some(task) = map.get_mut(&id) {
                match result {
                    Ok(SandboxResult { exit_code, stdout, stderr }) => {
                        task.output = stdout;
                        task.error_output = stderr;
                        task.exit_code = Some(exit_code);
                        task.progress = 100.0;
                        task.status = if exit_code == 0 {
                            TaskStatus::Completed
                        } else {
                            TaskStatus::Failed
                        };
                    }
                    Err(e) => {
                        task.error_output = e;
                        task.status = TaskStatus::Failed;
                    }
                }
            }
        });
    }

    pub fn get_sandbox(&self) -> &Sandbox {
        &self.sandbox
    }
}

/// Tasks I submitted to other machines (outgoing).
#[derive(Clone)]
pub struct OutgoingTasks {
    tasks: Arc<Mutex<HashMap<String, Task>>>,
}

impl OutgoingTasks {
    pub fn new() -> Self {
        Self {
            tasks: Arc::new(Mutex::new(HashMap::new())),
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

    pub fn remove(&self, id: &str) {
        self.tasks.lock().unwrap().remove(id);
    }
}
