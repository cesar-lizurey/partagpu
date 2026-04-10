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

        crate::user_manager::UserManager::create_user()?;

        crate::user_manager::UserManager::setup_cgroup(
            config.cpu_limit_percent,
            config.ram_limit_mb,
            config.gpu_limit_percent,
        )?;

        // Open firewall for incoming connections
        crate::user_manager::UserManager::open_port()?;

        config.status = SharingStatus::Active;
        Ok(config.clone())
    }

    pub fn disable(&self) -> Result<SharingConfig, String> {
        let mut config = self.config.lock().unwrap();

        // Close firewall — no more incoming connections
        let _ = crate::user_manager::UserManager::close_port();

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
