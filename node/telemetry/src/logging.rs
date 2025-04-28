use std::path::PathBuf;
use colored::Colorize;
use tracing::Level;
use tracing_appender::{
    non_blocking::WorkerGuard,
    rolling::{self, RollingFileAppender, Rotation},
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer};

/// Standard log file name prefix. This will be optionally appended with a timestamp
/// depending on the rotation strategy.
const LOG_FILE_NAME_PREFIX: &str = "1up.log";

pub struct LoggerConfig {
    pub log_level: Level,
    pub show_log_level: bool,
    pub show_crate_name: bool,
    pub show_time: bool,
    pub show_path: bool,
    pub logs_dir: Option<String>
}

impl Default for LoggerConfig {
    fn default() -> Self {
        Self {
            log_level: Level::INFO,
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
/// You can use the [crate::telemetry::init] function
/// to initialize a global logger, passing in `show_crate_name`, `show_time` and `show_path` parameters.
/// Function returns an error if a logger has already been initialized.
pub fn init_logger(config: LoggerConfig) -> Vec<WorkerGuard> {

    let LoggerConfig {
        log_level,
        show_log_level,
        show_crate_name,
        show_time,
        show_path,
        logs_dir
    } = config;

    // If a directory is provided, log to file and stdout
    if let Some(dir) = logs_dir {
        let directory = PathBuf::from(dir);
        let rotation = Rotation::DAILY;
        let appender = match rotation {
            Rotation::NEVER => rolling::never(directory, LOG_FILE_NAME_PREFIX),
            Rotation::DAILY => rolling::daily(directory, LOG_FILE_NAME_PREFIX),
            Rotation::HOURLY => rolling::hourly(directory, LOG_FILE_NAME_PREFIX),
            Rotation::MINUTELY => rolling::minutely(directory, LOG_FILE_NAME_PREFIX),
        };
        return build_subscriber(
            log_level,
            show_log_level,
            show_crate_name,
            show_time,
            show_path,
            Some(appender)
        );
    }
    // If no directory is provided, log to stdout only
    build_subscriber(
        log_level,
        show_log_level,
        show_crate_name,
        show_time,
        show_path,
        None
    )
}

/// Subscriber Composer
/// Builds a subscriber with multiple layers into a [tracing](https://crates.io/crates/tracing) subscriber
/// and initializes it as the global default. This subscriber will log to stdout and optionally to a file.
pub fn build_subscriber(
    log_level: Level,
    show_log_level: bool,
    show_crate_name: bool,
    show_time: bool,
    show_path: bool,
    appender: Option<RollingFileAppender>
) -> Vec<WorkerGuard> {
    let mut guards = Vec::new();

    let stdout_env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new(match log_level {
            Level::ERROR => "cmd=error,p2p_network=error,rpc=error,runtime=error".to_owned(),
            Level::WARN  => "cmd=warn,p2p_network=warn,rpc=warn,runtime=warn".to_owned(),
            Level::INFO  => "cmd=info,p2p_network=info,rpc=info,runtime=info".to_owned(),
            Level::DEBUG => "cmd=debug,p2p_network=debug,rpc=debug,runtime=debug".to_owned(),
            Level::TRACE => "cmd=trace,p2p_network=trace,rpc=trace,runtime=trace".to_owned(),
        })
    });
    // pub enum Level {
    //     Error = 1, // least verbose
    //     Warn = 2,
    //     Info = 3,
    //     Debug = 4,
    //     Trace = 5, // most verbose
    // }

    let stdout_formatting_layer = AnsiTermLayer {
        show_log_level,
        show_crate_name,
        show_time,
        show_path
    }.with_filter(stdout_env_filter);

    // If a file appender is provided, log to it and stdout, otherwise just log to stdout
    if let Some(appender) = appender {
        let (non_blocking, guard) = tracing_appender::non_blocking(appender);
        guards.push(guard);

        // Force the file logger to log at `debug` level
        let file_env_filter = EnvFilter::from("cmd=info,p2p-network=debug,rpc=debug,runtime=debug");

        tracing_subscriber::registry()
            .with(stdout_formatting_layer)
            .with(
                tracing_subscriber::fmt::layer()
                    .with_ansi(false)
                    .with_writer(non_blocking)
                    .with_filter(file_env_filter),
            )
            .init();
    } else {
        tracing_subscriber::registry()
            .with(stdout_formatting_layer)
            .init();
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

