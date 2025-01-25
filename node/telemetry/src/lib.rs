
/// The Logging Module
pub mod logging;

/// Prometheus metrics
pub mod metrics;

// Re-export inner modules
pub use logging::*;

/// Export a prelude to re-export common traits and types
pub mod prelude {
    pub use super::*;
    pub use tracing::{debug, error, info, span, trace, warn, Level};
    pub use tracing_subscriber::{fmt, prelude::*};
}