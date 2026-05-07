//! Tasks I submitted to other machines (outgoing dispatches).
//!
//! [`OutgoingTasks`] keeps a local mirror of every task we sent to a peer
//! so the UI can show one unified list (running, completed, cancelled)
//! without round-tripping per request. Mirroring is driven from
//! `http_api::run_remote_blocking` which polls the peer every 500 ms.
//!
//! Persistence layout : `~/.config/partagpu/outgoing-tasks.json` holds
//! both the task map and the `remote_refs` map (peer_ip + remote task id
//! per local id) — kept around so a cancel issued after a restart can
//! still propagate, although in practice the receiving peer has likely
//! also restarted.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use super::{config_dir, save_atomic, task_for_persist, Task, TaskStatus, PERSIST_FLUSH_INTERVAL};

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

impl Default for OutgoingTasks {
    fn default() -> Self {
        Self::new()
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
