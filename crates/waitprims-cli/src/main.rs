//! Diagnostic CLI for waitprims.
//!
//! Argument parsing and output only. Library crates own the logic.

use clap::{Parser, Subcommand, ValueEnum};
use tracing::info;

/// Diagnostic CLI for the waitprims library.
///
/// The library is the product. This binary is a local test vehicle.
/// There is no daemon.
#[derive(Parser, Debug)]
#[command(name = "waitprims", version, about, long_about = None)]
struct Cli {
    /// The format for log output (stderr).
    #[arg(long, value_name = "FORMAT", default_value = "text")]
    log_format: LogFormat,

    /// The minimum log level to display.
    #[arg(long, value_name = "LEVEL", default_value = "info")]
    log_level: tracing::Level,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Validate one message or a directory of messages.
    Validate,
    /// Replay a scripted live first-match wait.
    Wait,
    /// Replay a scripted poll cycle.
    Poll,
    /// Print bundled schema identifiers.
    Schema,
}

#[derive(ValueEnum, Clone, Debug, PartialEq, Eq)]
enum LogFormat {
    /// Human-readable text format.
    Text,
    /// Machine-readable JSON format.
    Json,
}

fn init_tracing(format: LogFormat, level: tracing::Level) {
    let builder = tracing_subscriber::fmt()
        .with_max_level(level)
        .with_writer(std::io::stderr)
        .with_target(false);

    match format {
        LogFormat::Text => builder.init(),
        LogFormat::Json => builder.json().init(),
    }
}

fn main() {
    let cli = Cli::parse();
    init_tracing(cli.log_format, cli.log_level);

    match cli.command {
        None => {
            println!(
                "waitprims {}
Diagnostic CLI. The library is the product; there is no daemon.",
                env!("CARGO_PKG_VERSION")
            );
        }
        Some(command) => {
            let name = match command {
                Command::Validate => "validate",
                Command::Wait => "wait",
                Command::Poll => "poll",
                Command::Schema => "schema",
            };
            info!(command = name, "subcommand is not implemented yet");
            eprintln!("waitprims {name}: not implemented yet");
            std::process::exit(1);
        }
    }
}
