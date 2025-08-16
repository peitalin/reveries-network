use std::error::Error;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, RwLock};
use std::process::{Command, Stdio};
use std::path::Path;
use color_eyre::{Result, eyre::anyhow};
use tracing::{info, error, debug};

/// Represents different reasons for restart
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub enum RestartReason {
    /// Triggered heartbeat failure for tests.
    ScheduledHeartbeatFailure,
    /// Triggered by network heartbeat failure.
    NetworkHeartbeatFailure,
    /// Generic error.
    Error(String),
}

/// Handles graceful shutdown and restart of the container
pub struct ContainerManager {
    shutdown_signal: broadcast::Sender<RestartReason>,
    is_shutting_down: AtomicBool,
    max_duration_before_shutdown: Duration,
    pub app_state: RwLock<AppState>,
}

impl ContainerManager {
    pub fn new(max_duration_before_shutdown: Duration) -> Self {
        let (shutdown_signal, _) = broadcast::channel(1);
        ContainerManager {
            shutdown_signal,
            is_shutting_down: AtomicBool::new(false),
            max_duration_before_shutdown,
            app_state: RwLock::new(AppState::default()),
        }
    }

    /// Subscribe to shutdown signals
    pub fn subscribe(&self) -> broadcast::Receiver<RestartReason> {
        self.shutdown_signal.subscribe()
    }

    /// Initiate graceful shutdown and restart
    pub async fn trigger_restart(&self, reason: RestartReason) -> Result<(), Box<dyn Error>> {
        // prevent multiple concurrent shutdown attempts
        if self.is_shutting_down.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        println!("ContainerManager: Initiating graceful shutdown due to: {:?}", reason);

        // Broadcast shutdown signal to all subsystems
        self.shutdown_signal.send(reason.clone()).ok();

        // Wait for pending operations to complete or timeout
        self.wait_for_graceful_shutdown().await?;

        // persist state if needed
        self.save_state().await?;

        // Exit the process to trigger container restart
        std::process::exit(0);
    }

    async fn wait_for_graceful_shutdown(&self) -> Result<(), Box<dyn Error>> {
        // Do something here to wait from pending operations to finish before shutting down
        Ok(())
    }

    async fn save_state(&self) -> Result<(), Box<dyn Error>> {
        // let state = self.app_state.clone();
        // Do something to save state: e.g save to an encrypted persistent volume
        Ok(())
    }

}

#[derive(Clone)]
pub struct AppState {
    pub healthy: bool,
    pub connections: u32,
    pub pending_operations: u32,
}
impl Default for AppState {
    fn default() -> Self {
        Self {
            healthy: true,
            connections: 0,
            pending_operations: 0,
        }
    }
}