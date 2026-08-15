//! Diagnostic CLI for waitprims.
//!
//! Argument parsing and output only. Library crates own the logic.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use tracing::info;
use waitprims_core::{
    resolve_bundled, validate_raw_documents, ValidationError, CAPABILITY, PINNED_CRUCIBLE_SHA,
};

/// Diagnostic CLI for the waitprims library.
///
/// The library is the product. This binary is a local test vehicle.
/// There is no daemon.
#[derive(Parser, Debug)]
#[command(name = "waitprims", version = env!("WAITPRIMS_VERSION"), about, long_about = None)]
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
    Validate {
        /// File or directory of `agent-wait/v0` JSON documents.
        #[arg(long, value_name = "PATH")]
        input: Option<PathBuf>,
        /// Positional alias for `--input`.
        #[arg(value_name = "PATH")]
        path: Option<PathBuf>,
    },
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

fn main() -> ExitCode {
    let cli = Cli::parse();
    init_tracing(cli.log_format, cli.log_level);

    match cli.command {
        None => {
            println!(
                "waitprims {}
Diagnostic CLI. The library is the product; there is no daemon.",
                env!("CARGO_PKG_VERSION")
            );
            ExitCode::SUCCESS
        }
        Some(Command::Schema) => match print_schema() {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("waitprims schema: {err}");
                ExitCode::from(1)
            }
        },
        Some(Command::Validate { input, path }) => match resolve_validate_target(input, path) {
            Ok(target) => match validate_path(&target) {
                Ok(count) => {
                    println!(
                        "{}",
                        serde_json::json!({
                            "ok": true,
                            "documents": count
                        })
                    );
                    ExitCode::SUCCESS
                }
                Err(err) => {
                    eprintln!("waitprims validate: {err}");
                    ExitCode::from(1)
                }
            },
            Err(err) => {
                eprintln!("waitprims validate: {err}");
                ExitCode::from(1)
            }
        },
        Some(command) => {
            let name = match command {
                Command::Wait => "wait",
                Command::Poll => "poll",
                Command::Validate { .. } | Command::Schema => unreachable!(),
            };
            info!(command = name, "subcommand is not implemented yet");
            eprintln!("waitprims {name}: not implemented yet");
            ExitCode::from(1)
        }
    }
}

fn resolve_validate_target(
    input: Option<PathBuf>,
    path: Option<PathBuf>,
) -> Result<PathBuf, waitprims_core::Error> {
    match (input, path) {
        (Some(target), None) | (None, Some(target)) => Ok(target),
        (Some(_), Some(_)) => Err(ValidationError::new("target", "input_and_positional").into()),
        (None, None) => Err(ValidationError::new("target", "missing").into()),
    }
}

fn print_schema() -> Result<(), waitprims_core::Error> {
    let resolved = resolve_bundled(CAPABILITY)?;
    println!(
        "{}",
        serde_json::json!({
            "capability": resolved.capability,
            "entry_schema": resolved.entry_schema_name,
            "crucible_sha": PINNED_CRUCIBLE_SHA
        })
    );
    Ok(())
}

fn validate_path(path: &Path) -> Result<usize, waitprims_core::Error> {
    reject_non_local_path(path)?;
    let documents = load_documents(path)?;
    let typed = validate_raw_documents(&documents)?;
    Ok(typed.len())
}

fn reject_non_local_path(path: &Path) -> Result<(), waitprims_core::Error> {
    let raw = path.as_os_str().to_string_lossy();
    if raw == "-" || looks_like_uri(&raw) {
        return Err(ValidationError::new("target", "local_path_required").into());
    }
    Ok(())
}

fn looks_like_uri(raw: &str) -> bool {
    raw.contains("://")
}

fn load_documents(path: &Path) -> Result<Vec<String>, waitprims_core::Error> {
    if path.is_file() {
        return Ok(vec![read_raw(path)?]);
    }
    if path.is_dir() {
        let mut files = Vec::new();
        collect_json(path, &mut files)?;
        files.sort();
        if files.is_empty() {
            return Err(ValidationError::new("/", "empty_target").into());
        }
        return files.iter().map(|p| read_raw(p)).collect();
    }
    Err(waitprims_core::Error::Contract {
        path: "target",
        constraint: "missing_or_unreadable",
    })
}

fn collect_json(dir: &Path, files: &mut Vec<PathBuf>) -> Result<(), waitprims_core::Error> {
    let entries = fs::read_dir(dir).map_err(|_| waitprims_core::Error::Contract {
        path: "target",
        constraint: "missing_or_unreadable",
    })?;
    for entry in entries {
        let entry = entry.map_err(|_| waitprims_core::Error::Contract {
            path: "target",
            constraint: "missing_or_unreadable",
        })?;
        let path = entry.path();
        if path.is_dir() {
            collect_json(&path, files)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("json") {
            files.push(path);
        }
    }
    Ok(())
}

fn read_raw(path: &Path) -> Result<String, waitprims_core::Error> {
    fs::read_to_string(path).map_err(|_| waitprims_core::Error::MalformedJson)
}
