//! Security event log — in-memory ring buffer accessible from the frontend.

use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// Maximum number of events kept in memory.
const MAX_EVENTS: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventLevel {
    Info,
    Warning,
    Alert,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventCategory {
    PeerConnected,
    PeerDisconnected,
    PeerVerified,
    PeerRejected,
    HostnameConflict,
    TaskSubmitted,
    TaskRejected,
    TaskCompleted,
    TaskFailed,
    RoomCreated,
    RoomJoined,
    RoomLeft,
    SharingEnabled,
    SharingDisabled,
    SharingPaused,
    FirewallOpened,
    FirewallClosed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityEvent {
    pub timestamp: u64,
    pub level: EventLevel,
    pub category: EventCategory,
    pub message: String,
    pub source_ip: Option<String>,
    pub source_host: Option<String>,
}

#[derive(Clone)]
pub struct SecurityLog {
    events: Arc<Mutex<Vec<SecurityEvent>>>,
}

impl SecurityLog {
    pub fn new() -> Self {
        Self {
            events: Arc::new(Mutex::new(Vec::with_capacity(MAX_EVENTS))),
        }
    }

    /// Log a security event.
    pub fn log(
        &self,
        level: EventLevel,
        category: EventCategory,
        message: &str,
        source_ip: Option<&str>,
        source_host: Option<&str>,
    ) {
        let event = SecurityEvent {
            timestamp: now_secs(),
            level,
            category,
            message: message.to_string(),
            source_ip: source_ip.map(|s| s.to_string()),
            source_host: source_host.map(|s| s.to_string()),
        };

        // Also print to stderr for system logs
        let level_str = match level {
            EventLevel::Info => "INFO",
            EventLevel::Warning => "WARN",
            EventLevel::Alert => "ALERT",
        };
        eprintln!("SECURITY [{level_str}] {:?}: {}", category, message);

        let mut events = self.events.lock().unwrap();
        if events.len() >= MAX_EVENTS {
            events.remove(0);
        }
        events.push(event);
    }

    /// Convenience: log an info event.
    pub fn info(&self, category: EventCategory, message: &str) {
        self.log(category.default_level(), category, message, None, None);
    }

    /// Convenience: log with peer info.
    pub fn peer_event(
        &self,
        category: EventCategory,
        message: &str,
        ip: &str,
        hostname: &str,
    ) {
        self.log(
            category.default_level(),
            category,
            message,
            Some(ip),
            Some(hostname),
        );
    }

    /// Get all events (newest last).
    pub fn get_all(&self) -> Vec<SecurityEvent> {
        self.events.lock().unwrap().clone()
    }

    /// Get events since a given timestamp.
    pub fn get_since(&self, since_timestamp: u64) -> Vec<SecurityEvent> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.timestamp > since_timestamp)
            .cloned()
            .collect()
    }

    /// Clear all events.
    pub fn clear(&self) {
        self.events.lock().unwrap().clear();
    }
}

impl EventCategory {
    fn default_level(self) -> EventLevel {
        match self {
            EventCategory::PeerConnected
            | EventCategory::PeerDisconnected
            | EventCategory::PeerVerified
            | EventCategory::TaskSubmitted
            | EventCategory::TaskCompleted
            | EventCategory::RoomCreated
            | EventCategory::RoomJoined
            | EventCategory::RoomLeft
            | EventCategory::SharingEnabled
            | EventCategory::SharingDisabled
            | EventCategory::SharingPaused
            | EventCategory::FirewallOpened
            | EventCategory::FirewallClosed => EventLevel::Info,

            EventCategory::PeerRejected
            | EventCategory::TaskFailed => EventLevel::Warning,

            EventCategory::HostnameConflict
            | EventCategory::TaskRejected => EventLevel::Alert,
        }
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
