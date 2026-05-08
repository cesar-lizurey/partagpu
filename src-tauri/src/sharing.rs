use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SharingStatus {
    Disabled,
    Active,
    Paused,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharingConfig {
    pub status: SharingStatus,
    pub cpu_limit_percent: u32,
    pub ram_limit_mb: u64,
    pub gpu_limit_percent: u32,
}

impl Default for SharingConfig {
    fn default() -> Self {
        Self {
            status: SharingStatus::Disabled,
            cpu_limit_percent: 50,
            ram_limit_mb: 0,
            gpu_limit_percent: 50,
        }
    }
}

#[derive(Clone)]
pub struct SharingController {
    config: Arc<Mutex<SharingConfig>>,
}

impl Default for SharingController {
    fn default() -> Self {
        Self::new()
    }
}

impl SharingController {
    pub fn new() -> Self {
        Self {
            config: Arc::new(Mutex::new(SharingConfig::default())),
        }
    }

    pub fn get_config(&self) -> SharingConfig {
        self.config.lock().unwrap().clone()
    }

    pub fn enable(&self) -> Result<SharingConfig, String> {
        let mut config = self.config.lock().unwrap();

        // Only create the user if it doesn't exist yet (requires pkexec)
        if !crate::user_manager::UserManager::user_exists() {
            crate::user_manager::UserManager::create_user()?;
        }

        // Setup cgroup — tries direct write first, falls back to pkexec
        crate::user_manager::UserManager::setup_cgroup(
            config.cpu_limit_percent,
            config.ram_limit_mb,
            config.gpu_limit_percent,
        )?;

        // Open firewall — may silently fail if already open or no privileges
        let _ = crate::user_manager::UserManager::open_port();

        // Start the CUDA MPS daemon so per-task `CUDA_MPS_ACTIVE_THREAD_PERCENTAGE`
        // env vars are honoured. Best-effort : a host without nvidia-cuda-mps-control
        // (no CUDA toolkit, or no NVIDIA GPU) silently degrades to advisory-only.
        let _ = crate::user_manager::UserManager::setup_mps();

        config.status = SharingStatus::Active;
        Ok(config.clone())
    }

    /// Désactivation complète : remove the partagpu user, kill its running
    /// processes, free the cgroup, close the firewall, drop the managed
    /// venv. Brings the system back to a clean state as if PartaGPU had
    /// never been activated. Use *Pause* for a temporary stop instead —
    /// pause keeps everything in place for fast resume.
    pub fn disable(&self) -> Result<SharingConfig, String> {
        let mut config = self.config.lock().unwrap();

        // Stop MPS first so its socket is gone before we wipe the user.
        let _ = crate::user_manager::UserManager::teardown_mps();

        // Full cleanup via the helper (pkexec password prompt). Kills all
        // running tasks (pkill -u partagpu), removes the user (userdel
        // --remove also wipes /var/lib/partagpu including the managed venv),
        // strips SSH/sudo deny rules, removes the restricted shell from
        // /etc/shells, deletes the cgroup, closes the firewall.
        crate::user_manager::UserManager::remove_user()?;

        config.status = SharingStatus::Disabled;
        Ok(config.clone())
    }

    pub fn pause(&self) -> Result<SharingConfig, String> {
        let mut config = self.config.lock().unwrap();
        if config.status != SharingStatus::Active {
            return Err("Cannot pause: sharing is not active".into());
        }

        // Close firewall during pause
        let _ = crate::user_manager::UserManager::close_port();

        config.status = SharingStatus::Paused;
        Ok(config.clone())
    }

    pub fn resume(&self) -> Result<SharingConfig, String> {
        let mut config = self.config.lock().unwrap();
        if config.status != SharingStatus::Paused {
            return Err("Cannot resume: sharing is not paused".into());
        }

        // Re-open firewall
        let _ = crate::user_manager::UserManager::open_port();

        config.status = SharingStatus::Active;
        Ok(config.clone())
    }

    /// Test-only : flip the in-memory status to Active without going through
    /// the helper / pkexec / cgroup setup. Used by the peer-API integration
    /// tests so they can submit tasks without requiring root privileges.
    #[doc(hidden)]
    pub fn force_active_for_tests(&self) {
        self.config.lock().unwrap().status = SharingStatus::Active;
    }

    pub fn set_limits(
        &self,
        cpu_percent: u32,
        ram_limit_mb: u64,
        gpu_percent: u32,
    ) -> Result<SharingConfig, String> {
        let mut config = self.config.lock().unwrap();
        config.cpu_limit_percent = cpu_percent.min(100);
        config.ram_limit_mb = ram_limit_mb;
        config.gpu_limit_percent = gpu_percent.min(100);

        if config.status == SharingStatus::Active {
            crate::user_manager::UserManager::setup_cgroup(
                config.cpu_limit_percent,
                config.ram_limit_mb,
                config.gpu_limit_percent,
            )?;
        }

        Ok(config.clone())
    }
}
