use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::sandbox::{Sandbox, SandboxResult};

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

    /// Execute a task inside the sandbox. Runs in a background thread.
    /// Updates the task status and output when done.
    pub fn execute(&self, task_id: &str, timeout_secs: u64) -> Result<(), String> {
        let tasks = self.tasks.clone();
        let sandbox = self.sandbox.clone();
        let id = task_id.to_string();

        // Get the task args
        let args = {
            let map = tasks.lock().unwrap();
            let task = map.get(&id).ok_or("Tâche introuvable.")?;
            if task.status != TaskStatus::Queued {
                return Err("La tâche n'est pas en attente.".into());
            }
            task.args.clone()
        };

        // Mark as running
        {
            let mut map = tasks.lock().unwrap();
            if let Some(task) = map.get_mut(&id) {
                task.status = TaskStatus::Running;
            }
        }

        // Execute in a thread
        std::thread::spawn(move || {
            let result = sandbox.execute(&args, timeout_secs);
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

    pub fn add(&self, task: Task) {
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

    pub fn remove(&self, id: &str) {
        self.tasks.lock().unwrap().remove(id);
    }
}
