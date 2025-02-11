use std::error::Error;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use serde::{Deserialize, Serialize};
use tokio::time::sleep;
use tokio::sync::broadcast;
use tokio::sync::RwLock;
use std::sync::Arc;


/// Represents different reasons for restart
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RestartReason {
    ScheduledHeartbeatFailure,
    HealthCheck,
    ResourceExhaustion,
    NetworkHeartbeatFailure,
    Error(String),
}

/// Handles graceful shutdown and restart of the container
pub struct ContainerManager {
    shutdown_signal: broadcast::Sender<RestartReason>,
    is_shutting_down: Arc<AtomicBool>,
    max_duration_before_shutdown: Duration,
    pub app_state: Arc<RwLock<AppState>>,
}

impl ContainerManager {

    pub fn new(max_duration_before_shutdown: Duration) -> Self {

        let (shutdown_signal, _) = broadcast::channel(1);

        ContainerManager {
            shutdown_signal,
            is_shutting_down: Arc::new(AtomicBool::new(false)),
            max_duration_before_shutdown,
            app_state: Arc::new(RwLock::new(AppState::default())),
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
        // let start = std::time::Instant::now();
        // let state = self.app_state.clone();

        // while start.elapsed() < self.max_duration_before_shutdown {
        //     let current_state = state.read().await;
        //     if current_state.pending_operations == 0 && current_state.connections == 0 {
        //         return Ok(());
        //     }
        //     println!(
        //         "Waiting for shutdown: {} pending operations, {} connections",
        //         current_state.pending_operations, current_state.connections
        //     );
        //     sleep(Duration::from_secs(1)).await;
        // }

        // println!("Shutdown timeout reached, forcing restart");
        Ok(())
    }

    async fn save_state(&self) -> Result<(), Box<dyn Error>> {
        // let state = self.app_state.clone();
        // let save_state = state.read().await;

        // TODO
        // Save to an ecrypted persistent volume or on MPC network
        // encrypt and save somewhere threshold decryptable by MPC network
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