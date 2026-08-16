//! Diagnostic CLI for waitprims.
//!
//! Argument parsing and output only. Library crates own the logic.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use tracing::info;
use waitprims_async::{run_first_match, run_poll_cycle, Cancel};
use waitprims_core::{
    resolve_bundled, validate_message, validate_raw_documents, AgentWaitMessage, Error,
    LiveWaitRequest, PollCycleRequest, RegistrationSet, ValidationError, CAPABILITY,
    PINNED_CRUCIBLE_SHA,
};
use waitprims_testkit::{FakeClock, Script, ScriptedObserver};

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
        input: PathBuf,
    },
    /// Replay a scripted live first-match wait.
    Wait {
        /// Admitted `registration_set` JSON file.
        #[arg(long, value_name = "PATH")]
        registration_set: PathBuf,
        /// Admitted `live_wait_request` JSON file.
        #[arg(long, value_name = "PATH")]
        request: PathBuf,
        /// Local scripted events JSON file.
        #[arg(long, value_name = "PATH")]
        script: PathBuf,
    },
    /// Replay a scripted poll cycle.
    Poll {
        /// Admitted `registration_set` JSON file.
        #[arg(long, value_name = "PATH")]
        registration_set: PathBuf,
        /// Admitted `poll_cycle_request` JSON file.
        #[arg(long, value_name = "PATH")]
        request: PathBuf,
        /// Local scripted events JSON file.
        #[arg(long, value_name = "PATH")]
        script: PathBuf,
    },
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
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => {
            let _ = err.print();
            return if err.use_stderr() {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            };
        }
    };
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
        Some(Command::Validate { input }) => match validate_path(&input) {
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
        Some(Command::Wait {
            registration_set,
            request,
            script,
        }) => match run_wait(&registration_set, &request, &script) {
            Ok(json) => {
                println!("{json}");
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("waitprims wait: {err}");
                ExitCode::from(1)
            }
        },
        Some(Command::Poll {
            registration_set,
            request,
            script,
        }) => match run_poll(&registration_set, &request, &script) {
            Ok(json) => {
                println!("{json}");
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("waitprims poll: {err}");
                ExitCode::from(1)
            }
        },
    }
}

fn run_wait(set_path: &Path, request_path: &Path, script_path: &Path) -> Result<String, Error> {
    reject_non_local_path(set_path)?;
    reject_non_local_path(request_path)?;
    reject_non_local_path(script_path)?;
    let set_raw = read_raw(set_path)?;
    let request_raw = read_raw(request_path)?;
    let script_raw = read_raw(script_path)?;
    let admitted = validate_raw_documents([&set_raw, &request_raw])?;
    let (set, request) = take_set_and_request(admitted)?;
    let script = Script::from_json(&script_raw)?;
    for event in &script.events {
        if !set
            .registrations
            .iter()
            .any(|reg| reg.registration_id.as_str() == event.registration_id.as_str())
        {
            return Err(
                ValidationError::new("/events/registration_id", "unknown_registration").into(),
            );
        }
    }
    info!("running scripted first-match");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .map_err(|_| Error::Contract {
            path: "runtime",
            constraint: "init",
        })?;
    runtime.block_on(async {
        let clock = FakeClock::auto(request.created_at.clone());
        let observer = ScriptedObserver::new(script, clock.clone());
        let cancel = Cancel::new();
        let outcome = run_first_match(&set, &request, &observer, &clock, &cancel).await?;
        let kind = outcome.outcome_kind.as_str();
        info!(outcome_kind = kind, "emitted live_wait_outcome");
        let message = AgentWaitMessage::LiveWaitOutcome(outcome);
        let json = serde_json::to_string(&message).map_err(|_| Error::MalformedJson)?;
        validate_message(&json)?;
        Ok(json)
    })
}

fn run_poll(set_path: &Path, request_path: &Path, script_path: &Path) -> Result<String, Error> {
    reject_non_local_path(set_path)?;
    reject_non_local_path(request_path)?;
    reject_non_local_path(script_path)?;
    let set_raw = read_raw(set_path)?;
    let request_raw = read_raw(request_path)?;
    let script_raw = read_raw(script_path)?;
    let admitted = validate_raw_documents([&set_raw, &request_raw])?;
    let (set, request) = take_set_and_poll_request(admitted)?;
    let script = Script::from_json(&script_raw)?;
    for event in &script.events {
        if !set
            .registrations
            .iter()
            .any(|reg| reg.registration_id.as_str() == event.registration_id.as_str())
        {
            return Err(
                ValidationError::new("/events/registration_id", "unknown_registration").into(),
            );
        }
    }
    info!("running scripted poll cycle");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .map_err(|_| Error::Contract {
            path: "runtime",
            constraint: "init",
        })?;
    runtime.block_on(async {
        let clock = FakeClock::auto(request.created_at.clone());
        let observer = ScriptedObserver::new(script, clock.clone());
        let cancel = Cancel::new();
        let outcome = run_poll_cycle(&set, &request, &observer, &clock, &cancel).await?;
        let kind = outcome.outcome_kind.as_str();
        info!(outcome_kind = kind, "emitted poll_cycle_outcome");
        let message = AgentWaitMessage::PollCycleOutcome(outcome);
        let json = serde_json::to_string(&message).map_err(|_| Error::MalformedJson)?;
        validate_message(&json)?;
        Ok(json)
    })
}

fn take_set_and_poll_request(
    admitted: Vec<waitprims_core::AdmittedMessage>,
) -> Result<(RegistrationSet, PollCycleRequest), Error> {
    let mut set = None;
    let mut request = None;
    for message in admitted {
        match message.into_inner() {
            AgentWaitMessage::RegistrationSet(value) => set = Some(value),
            AgentWaitMessage::PollCycleRequest(value) => request = Some(value),
            _ => {
                return Err(ValidationError::new("/message_type", "unexpected_kind").into());
            }
        }
    }
    let set =
        set.ok_or_else(|| ValidationError::new("/message_type", "registration_set_required"))?;
    let request = request
        .ok_or_else(|| ValidationError::new("/message_type", "poll_cycle_request_required"))?;
    Ok((set, request))
}

fn take_set_and_request(
    admitted: Vec<waitprims_core::AdmittedMessage>,
) -> Result<(RegistrationSet, LiveWaitRequest), Error> {
    let mut set = None;
    let mut request = None;
    for message in admitted {
        match message.into_inner() {
            AgentWaitMessage::RegistrationSet(value) => set = Some(value),
            AgentWaitMessage::LiveWaitRequest(value) => request = Some(value),
            _ => {
                return Err(ValidationError::new("/message_type", "unexpected_kind").into());
            }
        }
    }
    let set =
        set.ok_or_else(|| ValidationError::new("/message_type", "registration_set_required"))?;
    let request = request
        .ok_or_else(|| ValidationError::new("/message_type", "live_wait_request_required"))?;
    Ok((set, request))
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
