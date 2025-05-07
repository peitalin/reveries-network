use std::path::PathBuf;
use tracing::Level;
use tracing_appender::{
    non_blocking::WorkerGuard,
    rolling,
};
use tracing_subscriber::{
    layer::SubscriberExt,
    util::SubscriberInitExt,
    EnvFilter,
    Layer,
    fmt,
    registry
};
use std::env; // Added for reading TEST_ENV

// For TestWriter
use tracing_subscriber::fmt::TestWriter;

const LOG_FILE_NAME_PREFIX: &str = "reveries.log";

pub struct LoggerConfig {
    pub show_log_level: bool,
    pub show_crate_name: bool,
    pub show_time: bool,
    pub show_path: bool,
    pub logs_dir: Option<String>
}

impl Default for LoggerConfig {
    fn default() -> Self {
        Self {
            show_log_level: true,
            show_crate_name: false,
            show_time: false,
            show_path: true,
            logs_dir: None
        }
    }
}

/// Logging
/// Configure logging telemetry with a global handler.
/// Function returns an error if a logger has already been initialized.
pub fn init_logger(config: LoggerConfig) -> Vec<WorkerGuard> {
    let is_test_env = env::var("TEST_ENV").unwrap_or_default().to_lowercase() == "true";

    let default_filter_str = if is_test_env {
        "info,cmd=debug,p2p_network=debug,rpc=debug,runtime=debug,telemetry=debug,tests_e2e=debug,llm_proxy=debug"
    } else {
        "info,cmd=debug,p2p_network=debug,rpc=debug,runtime=debug,telemetry=debug,llm_proxy=debug"
    };

    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(default_filter_str));

    let mut guards = Vec::new();

    if is_test_env {
        let subscriber_builder = fmt::Subscriber::builder()
            .with_max_level(Level::DEBUG)
            .with_env_filter(env_filter)
            .with_writer(TestWriter::new())
            .with_target(config.show_path)
            .with_level(config.show_log_level)
            .without_time();

        let subscriber = subscriber_builder.finish();

        tracing::subscriber::set_global_default(subscriber)
            .map_err(|e| eprintln!("Failed to set global default subscriber for tests: {}", e))
            .ok();

        tracing::info!("Tracing logger (for tests with TestWriter) initialized");

    } else {
        // --- Non-Test Environment: Use your AnsiTermLayer and optional file logging ---
        let stdout_layer = AnsiTermLayer {
            show_log_level: config.show_log_level,
            show_crate_name: config.show_crate_name,
            show_time: config.show_time,
            show_path: config.show_path,
        }.with_filter(env_filter);

        if let Some(dir) = config.logs_dir {
            let directory = PathBuf::from(dir);
            let file_appender = rolling::daily(directory, LOG_FILE_NAME_PREFIX);
            let (non_blocking_file, guard) = tracing_appender::non_blocking(file_appender);
            guards.push(guard);

            let file_filter = EnvFilter::try_from_env("RUST_LOG_FILE")
                .unwrap_or_else(|_| EnvFilter::new("debug"));

            let file_layer = fmt::layer()
                .with_ansi(false)
                .with_writer(non_blocking_file)
                .with_filter(file_filter);

            registry()
                .with(stdout_layer)
                .with(file_layer)
                .init();
            tracing::info!("Tracing logger (File) initialized with filter by RUST_LOG");
        } else {
            registry()
                .with(stdout_layer)
                .init();
            tracing::info!("Tracing logger initialized with filter by RUST_LOG");
        }
    }
    guards
}

/// The AnsiVisitor
#[derive(Debug)]
pub struct AnsiVisitor;

impl tracing::field::Visit for AnsiVisitor {
    fn record_f64(&mut self, _: &tracing::field::Field, value: f64) {
        println!("{value}")
    }

    fn record_i64(&mut self, _: &tracing::field::Field, value: i64) {
        println!("{value}")
    }

    fn record_u64(&mut self, _: &tracing::field::Field, value: u64) {
        println!("{value}")
    }

    fn record_bool(&mut self, _: &tracing::field::Field, value: bool) {
        println!("{value}")
    }

    fn record_str(&mut self, _: &tracing::field::Field, value: &str) {
        println!("{value}")
    }

    fn record_error(
        &mut self,
        _: &tracing::field::Field,
        value: &(dyn std::error::Error + 'static),
    ) {
        println!("{value}")
    }

    fn record_debug(&mut self, _: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        println!("{value:?}")
    }
}

/// An Ansi Term layer for tracing
#[derive(Debug)]
pub struct AnsiTermLayer {
    /// Whether to show log level prefixes in logs (INFO, DEBUG, WARN, ERROR, TRACE)
    show_log_level: bool,
    /// Whether show_crate_name tracing is enabled. Prints additional metadata if `true`
    show_crate_name: bool,
    /// Whether to show timestamp in logs
    show_time: bool,
    /// Whether to show line/path the event was emitted in logs
    show_path: bool,
}

impl<S> Layer<S> for AnsiTermLayer
    where S: tracing::Subscriber,
{
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        use colored::Colorize;
        if self.show_time {
            // Print the timestamp
            let utc: String = chrono::Utc::now().to_rfc3339();
            // trim milliseconds
            let strip_len = utc.find('.').unwrap_or(utc.len());
            let formatted_utc = utc[..strip_len].trim_end().replace("T", " ");
            print!("[{}]", formatted_utc.blue());
        }

        // Print the level prefix
        if self.show_log_level {
            match *event.metadata().level() {
                Level::ERROR => {
                    print!("{}", format!("[{}]", "ERROR").red());
                }
                Level::WARN => {
                    print!("{}", format!("[{}]", "WARN").yellow());
                }
                Level::INFO => {
                    print!("{}", format!("[{}]", "INFO").cyan());
                }
                Level::DEBUG => {
                    print!("{}", format!("[{}]", "DEBUG").cyan());
                }
                Level::TRACE => {
                    print!("{}", format!("[{}]", "TRACE").purple());
                }
            }
        }

        if self.show_crate_name {
            let crate_name = event.metadata().target();
            print!("{}", format!("[{}]", crate_name).bright_blue());
        }

        if self.show_path {
            let original_location = event
                .metadata()
                .name()
                .split(' ')
                .last()
                .unwrap_or_default();

            if self.show_crate_name {
                // show full filepath
                print!("{}", format!("[{}]", original_location).blue());
            } else {
                // show short filepath
                let location = original_location
                    .split('/')
                    .rev()
                    .take(2)
                    .collect::<Vec<&str>>()
                    .into_iter()
                    .rev()
                    .collect::<Vec<&str>>()
                    .join("/");

                print!("{}", format!("[{}]", location).blue());
            }
        }

        // extra space
        print!(" ");

        let mut visitor = AnsiVisitor;
        event.record(&mut visitor);
    }
}

