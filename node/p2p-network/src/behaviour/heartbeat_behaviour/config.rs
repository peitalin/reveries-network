use std::time::Duration;

#[derive(Debug, Clone)]
pub struct HeartbeatConfig {
    /// Sending of `TeeAttestation` should not take longer than this
    pub(crate) send_timeout: Duration,
    /// Idle time before sending next `TeeAttestation`
    pub(crate) idle_timeout: Duration,
    /// Max failures allowed.
    /// If reached `HeartbeatHandler` will request closing of the connection.
    pub(crate) max_failures: u32,
}

impl HeartbeatConfig {
    pub fn new(
        send_timeout: Duration,
        idle_timeout: Duration,
        max_failures: u32,
    ) -> Self {
        Self {
            send_timeout,
            idle_timeout,
            max_failures,
        }
    }

    pub fn max_time_before_rotation(&self) -> Duration {
        self.send_timeout * self.max_failures.into()
    }
}

impl Default for HeartbeatConfig {
    fn default() -> Self {
        Self {
            send_timeout: Duration::from_secs(60),
            idle_timeout: Duration::from_secs(1),
            max_failures: 5,
        }
    }
}