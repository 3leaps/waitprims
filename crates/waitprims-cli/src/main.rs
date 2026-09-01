//! Diagnostic CLI for waitprims.
//!
//! Argument parsing and output only. Library crates own the logic.
//! There is no daemon, no credential flag, and no extra wire kind.
//! JSON goes to stdout; logs and errors go to stderr.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Instant;

mod diagnostic;

use std::time::Duration;

use clap::{Parser, Subcommand, ValueEnum};
use tracing::info;
use waitprims_async::{
    run_coalesce, run_first_match, run_follow, run_poll_cycle, Cancel, Clock, CoalescePolicy,
};
use waitprims_core::{
    bundled_entry_schema, bundled_message_schema, validate_message, validate_raw_documents,
    AgentWaitMessage, ContentDigest, DigestAlgorithm, Error, LiveWaitRequest, MessageType,
    OpaqueRef, PayloadRef, PollCycleRequest, RegistrationSet, Timestamp, ValidationError,
};
use waitprims_fs::{
    EventClock, EventDescriptor, EventRefSink, FilesystemPosture, FsObserver, SystemEventClock,
    METHOD_FILE_WATCH,
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
    /// Replay a scripted held-follow session.
    Follow {
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
    /// Follow native local-filesystem events.
    Watch {
        /// Existing local directory that owns all watched subjects.
        #[arg(long, value_name = "DIR")]
        root: PathBuf,
        /// Admitted `registration_set` JSON file.
        #[arg(long, value_name = "PATH")]
        registration_set: PathBuf,
        /// Admitted `live_wait_request` JSON file.
        #[arg(long, value_name = "PATH")]
        request: PathBuf,
    },
    /// Replay a scripted held-coalesce session.
    Coalesce {
        /// Admitted `registration_set` JSON file.
        #[arg(long, value_name = "PATH")]
        registration_set: PathBuf,
        /// Admitted `live_wait_request` JSON file.
        #[arg(long, value_name = "PATH")]
        request: PathBuf,
        /// Local scripted events JSON file.
        #[arg(long, value_name = "PATH")]
        script: PathBuf,
        /// Minimum gap between quiet emits.
        #[arg(long, value_name = "DURATION")]
        min_emit_interval: Option<String>,
        /// Effective priority at or above this flushes immediately (0-255).
        #[arg(long, value_name = "0-255")]
        urgent_at: Option<u8>,
    },
    /// Print the compiled contract pin.
    Contract,
    /// Print the bundled JSON Schema, or one message kind's definition.
    Schema {
        /// Restrict output to one of the six `message_type` values.
        #[arg(long, value_name = "KIND")]
        message_type: Option<String>,
    },
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
        Some(Command::Schema { message_type }) => match print_schema(message_type.as_deref()) {
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
        Some(Command::Follow {
            registration_set,
            request,
            script,
        }) => match run_held_follow(&registration_set, &request, &script) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("waitprims follow: {err}");
                ExitCode::from(1)
            }
        },
        Some(Command::Watch {
            root,
            registration_set,
            request,
        }) => match run_native_watch(&root, &registration_set, &request) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("waitprims watch: {err}");
                ExitCode::from(1)
            }
        },
        Some(Command::Coalesce {
            registration_set,
            request,
            script,
            min_emit_interval,
            urgent_at,
        }) => match run_held_coalesce(
            &registration_set,
            &request,
            &script,
            min_emit_interval.as_deref(),
            urgent_at,
        ) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("waitprims coalesce: {err}");
                ExitCode::from(1)
            }
        },
        Some(Command::Contract) => match print_contract() {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("waitprims contract: {err}");
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

fn run_held_follow(set_path: &Path, request_path: &Path, script_path: &Path) -> Result<(), Error> {
    reject_non_local_path(set_path)?;
    reject_non_local_path(request_path)?;
    reject_non_local_path(script_path)?;
    let set_raw = read_raw(set_path)?;
    let request_raw = read_raw(request_path)?;
    let admitted = validate_raw_documents([&set_raw, &request_raw])?;
    let (set, request) = take_set_and_request(admitted)?;
    let script_raw = read_raw(script_path)?;
    let script = Script::from_json(&script_raw)?;
    reject_unknown_registrations(&set, &script)?;
    info!("running scripted follow");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .map_err(|_| Error::Contract {
            path: "runtime",
            constraint: "init",
        })?;
    let mut sink = diagnostic::JsonlSink::new(std::io::stdout());
    let end = runtime.block_on(async {
        let clock = FakeClock::auto(request.created_at.clone());
        let observer = ScriptedObserver::new(script, clock.clone());
        let cancel = Cancel::new();
        run_follow(&observer, &clock, &cancel, &set, &request, |burst| {
            let result = sink.emit_burst(&burst);
            async move { result }
        })
        .await
    })?;
    sink.emit_end(&end)?;
    Ok(())
}

fn run_native_watch(root: &Path, set_path: &Path, request_path: &Path) -> Result<(), Error> {
    let (root, set, request, source_instance_ref) =
        admit_watch_inputs(root, set_path, request_path)?;
    let event_sink = std::sync::Arc::new(CliEventSink::new(
        set.aggregate_limits.max_events,
        set.aggregate_limits.max_bytes,
    ));
    let observer = FsObserver::new(
        source_instance_ref,
        root,
        FilesystemPosture::Local,
        event_sink.clone(),
    )?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .map_err(|_| Error::Contract {
            path: "runtime",
            constraint: "init",
        })?;
    let clock = RealtimeClock::new();
    let cancel = Cancel::new();
    let mut sink = diagnostic::JsonlSink::new(std::io::stdout());
    let end = runtime.block_on(run_follow(
        &observer,
        &clock,
        &cancel,
        &set,
        &request,
        |burst| {
            let result = sink.emit_burst(&burst);
            async move { result }
        },
    ))?;
    if event_sink.failed() {
        return Err(Error::Contract {
            path: "event_ref_sink",
            constraint: "capacity",
        });
    }
    sink.emit_end(&end)
}

fn admit_watch_inputs(
    root: &Path,
    set_path: &Path,
    request_path: &Path,
) -> Result<(PathBuf, RegistrationSet, LiveWaitRequest, OpaqueRef), Error> {
    reject_non_local_path(root)?;
    reject_non_local_path(set_path)?;
    reject_non_local_path(request_path)?;
    let root = fs::canonicalize(root)
        .map_err(|_| ValidationError::new("/root", "existing_local_directory_required"))?;
    if !root.is_dir() {
        return Err(ValidationError::new("/root", "existing_local_directory_required").into());
    }

    let set_raw = read_raw(set_path)?;
    let request_raw = read_raw(request_path)?;
    let admitted = validate_raw_documents([&set_raw, &request_raw])?;
    let (set, request) = take_set_and_request(admitted)?;
    let source_instance_ref = set
        .registrations
        .first()
        .ok_or_else(|| ValidationError::new("/registrations", "nonempty_required"))?
        .source_instance_ref
        .clone();
    for registration in &set.registrations {
        if registration.method_id.as_str() != METHOD_FILE_WATCH {
            return Err(
                ValidationError::new("/registrations/method_id", "file_watch_required").into(),
            );
        }
        if registration.source_instance_ref != source_instance_ref {
            return Err(ValidationError::new(
                "/registrations/source_instance_ref",
                "single_source_required",
            )
            .into());
        }
        if registration.subject_kind.as_str() != "path" {
            return Err(
                ValidationError::new("/registrations/subject_kind", "path_required").into(),
            );
        }
        validate_watch_subject(registration.subject_id.as_str())?;
    }
    Ok((root, set, request, source_instance_ref))
}

fn validate_watch_subject(subject: &str) -> Result<(), Error> {
    use std::path::Component;

    if subject.is_empty()
        || subject.starts_with('/')
        || subject.starts_with('\\')
        || subject.contains('\\')
        || subject.as_bytes().get(1) == Some(&b':')
    {
        return Err(ValidationError::new(
            "/registrations/subject_id",
            "portable_relative_path_required",
        )
        .into());
    }
    if Path::new(subject).components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err(ValidationError::new(
            "/registrations/subject_id",
            "portable_relative_path_required",
        )
        .into());
    }
    Ok(())
}

struct RealtimeClock {
    timestamp: Timestamp,
    instant: Instant,
}

impl RealtimeClock {
    fn new() -> Self {
        Self {
            timestamp: SystemEventClock.now(),
            instant: Instant::now(),
        }
    }
}

impl Clock for RealtimeClock {
    fn now(&self) -> Timestamp {
        self.timestamp.saturating_add(self.instant.elapsed())
    }

    async fn sleep_until(&self, deadline: &Timestamp) {
        tokio::time::sleep(self.now().duration_until(deadline)).await;
    }
}

struct CliEventSink {
    next: AtomicU64,
    descriptors: Mutex<CliDescriptorStore>,
    max_events: u64,
    max_bytes: u64,
    failed: AtomicBool,
    #[cfg(test)]
    inspector: Option<DescriptorInspector>,
}

#[derive(Default)]
struct CliDescriptorStore {
    bytes: u64,
    entries: std::collections::BTreeMap<String, Vec<u8>>,
}

#[cfg(test)]
type DescriptorInspector = std::sync::Arc<dyn Fn(&EventDescriptor, &[u8], &str, u64) + Send + Sync>;

impl CliEventSink {
    fn new(max_events: u64, max_bytes: u64) -> Self {
        Self {
            next: AtomicU64::new(0),
            descriptors: Mutex::new(CliDescriptorStore::default()),
            max_events,
            max_bytes,
            failed: AtomicBool::new(false),
            #[cfg(test)]
            inspector: None,
        }
    }

    fn failed(&self) -> bool {
        self.failed.load(Ordering::Acquire)
    }
}

impl Default for CliEventSink {
    fn default() -> Self {
        Self::new(u64::MAX, u64::MAX)
    }
}

impl EventRefSink for CliEventSink {
    fn materialize(&self, descriptor: &EventDescriptor) -> waitprims_core::Result<PayloadRef> {
        use sha2::{Digest, Sha256};

        let bytes = descriptor.canonical_bytes();
        let digest = Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        #[cfg(test)]
        if let Some(inspector) = &self.inspector {
            inspector(
                descriptor,
                &bytes,
                &digest,
                self.next.load(Ordering::Acquire),
            );
        }
        let sequence = self.next.fetch_add(1, Ordering::AcqRel) + 1;
        let payload_ref = format!("ref:fs-cli-{sequence}");
        let mut store = self.descriptors.lock().expect("CLI descriptor store");
        let next_bytes = store.bytes.saturating_add(bytes.len() as u64);
        if sequence > self.max_events || next_bytes > self.max_bytes {
            self.failed.store(true, Ordering::Release);
            return Err(ValidationError::new("/event_ref_sink", "capacity_exhausted").into());
        }
        store.bytes = next_bytes;
        store.entries.insert(payload_ref.clone(), bytes);
        Ok(PayloadRef {
            payload_ref: OpaqueRef::new(payload_ref),
            content_digest: ContentDigest {
                algorithm: DigestAlgorithm::Sha256,
                value: digest,
            },
            media_type: Some("application/json".to_string()),
        })
    }
}

fn run_held_coalesce(
    set_path: &Path,
    request_path: &Path,
    script_path: &Path,
    min_emit_interval: Option<&str>,
    urgent_at: Option<u8>,
) -> Result<(), Error> {
    reject_non_local_path(set_path)?;
    reject_non_local_path(request_path)?;
    reject_non_local_path(script_path)?;
    let set_raw = read_raw(set_path)?;
    let request_raw = read_raw(request_path)?;
    let admitted = validate_raw_documents([&set_raw, &request_raw])?;
    let (set, request) = take_set_and_request(admitted)?;
    let script_raw = read_raw(script_path)?;
    let script = Script::from_json(&script_raw)?;
    reject_unknown_registrations(&set, &script)?;
    let mut policy = match min_emit_interval {
        Some(raw) => CoalescePolicy::new(parse_duration(raw)?),
        None => CoalescePolicy::new(Duration::from_secs(10)),
    };
    if let Some(at) = urgent_at {
        policy.urgent_at = at;
    }
    info!("running scripted coalesce");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .map_err(|_| Error::Contract {
            path: "runtime",
            constraint: "init",
        })?;
    let mut sink = diagnostic::JsonlSink::new(std::io::stdout());
    let end = runtime.block_on(async {
        let clock = FakeClock::auto(request.created_at.clone());
        let observer = ScriptedObserver::new(script, clock.clone());
        let cancel = Cancel::new();
        run_coalesce(
            &observer,
            &clock,
            &cancel,
            &set,
            &request,
            &policy,
            |burst| {
                let result = sink.emit_coalesce_burst(&burst);
                async move { result }
            },
        )
        .await
    })?;
    sink.emit_end(&end)?;
    Ok(())
}

fn parse_duration(raw: &str) -> Result<Duration, Error> {
    let body = raw.trim();
    let (amount, unit) = if let Some(rest) = body.strip_suffix("ms") {
        (rest, 0)
    } else if let Some(rest) = body.strip_suffix('s') {
        (rest, 1)
    } else if let Some(rest) = body.strip_suffix('m') {
        (rest, 2)
    } else if let Some(rest) = body.strip_suffix('h') {
        (rest, 3)
    } else {
        (body, 1)
    };
    let value: u64 = amount
        .trim()
        .parse()
        .map_err(|_| ValidationError::new("/min_emit_interval", "invalid_duration"))?;
    match unit {
        // Preserve sub-second precision for `ms`; do not truncate to seconds.
        0 => Ok(Duration::from_millis(value)),
        1 => Ok(Duration::from_secs(value)),
        2 => Ok(Duration::from_secs(value.saturating_mul(60))),
        3 => Ok(Duration::from_secs(value.saturating_mul(3600))),
        _ => unreachable!(),
    }
}

fn print_contract() -> Result<(), Error> {
    let json = diagnostic::contract_json()?;
    println!("{json}");
    Ok(())
}

fn reject_unknown_registrations(set: &RegistrationSet, script: &Script) -> Result<(), Error> {
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
    Ok(())
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

fn print_schema(message_type: Option<&str>) -> Result<(), waitprims_core::Error> {
    let schema = match message_type {
        None => bundled_entry_schema()?,
        Some(raw) => {
            let kind = MessageType::parse(raw)
                .ok_or_else(|| ValidationError::new("/message_type", "undeclared_message_type"))?;
            bundled_message_schema(kind)?
        }
    };
    let json = serde_json::to_string(&schema).map_err(|_| Error::MalformedJson)?;
    println!("{json}");
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
    let Some((scheme, rest)) = raw.split_once(':') else {
        return false;
    };
    if !is_uri_scheme(scheme) {
        return false;
    }
    // A single-letter scheme plus a path separator is a drive, not a URI.
    if scheme.len() == 1 && (rest.starts_with('/') || rest.starts_with('\\')) {
        return false;
    }
    true
}

fn is_uri_scheme(scheme: &str) -> bool {
    let mut chars = scheme.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_alphabetic()
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
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

#[cfg(test)]
mod tests {
    use super::{
        looks_like_uri, parse_duration, reject_non_local_path, validate_watch_subject,
        CliEventSink, DescriptorInspector,
    };
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use waitprims_fs::{EventDescriptor, EventRefSink, FileEventClass};

    #[test]
    fn millisecond_duration_is_preserved_not_truncated() {
        use std::time::Duration;
        assert_eq!(
            parse_duration("500ms").expect("500ms"),
            Duration::from_millis(500)
        );
        assert_eq!(
            parse_duration("1500ms").expect("1500ms"),
            Duration::from_millis(1500)
        );
        assert_eq!(parse_duration("1s").expect("1s"), Duration::from_secs(1));
        assert_eq!(parse_duration("10").expect("10"), Duration::from_secs(10));
        assert_eq!(parse_duration("2m").expect("2m"), Duration::from_secs(120));
        assert_eq!(parse_duration("1h").expect("1h"), Duration::from_secs(3600));
    }

    #[test]
    fn invalid_duration_is_rejected() {
        assert!(parse_duration("abc").is_err());
        assert!(parse_duration("12x").is_err());
    }

    #[test]
    fn ordinary_paths_are_local() {
        for raw in [
            "fixtures/initial-case/live.json",
            "./registration_set.json",
            "../fixtures/initial-case/poll.json",
            "/var/tmp/waitprims/request.json",
            r"C:\temp\request.json",
        ] {
            assert!(!looks_like_uri(raw), "{raw} must remain a filesystem path");
            reject_non_local_path(Path::new(raw)).expect(raw);
        }
    }

    #[test]
    fn cli_sink_inspects_descriptor_and_digest_before_ref_creation() {
        use sha2::{Digest, Sha256};

        let inspected = Arc::new(AtomicBool::new(false));
        let callback_inspected = inspected.clone();
        let inspector: DescriptorInspector =
            Arc::new(move |descriptor, bytes, digest, refs_created| {
                assert_eq!(refs_created, 0, "inspection must precede ref creation");
                assert_eq!(descriptor.class, FileEventClass::Create);
                assert_eq!(descriptor.paths, ["watched/leaf"]);
                assert_eq!(bytes, descriptor.canonical_bytes());
                let expected = Sha256::digest(bytes)
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>();
                assert_eq!(digest, expected);
                callback_inspected.store(true, Ordering::Release);
            });
        let sink = CliEventSink {
            inspector: Some(inspector),
            ..CliEventSink::default()
        };
        let payload = sink
            .materialize(&EventDescriptor {
                class: FileEventClass::Create,
                paths: vec!["watched/leaf".to_string()],
            })
            .expect("materialize");

        assert!(inspected.load(Ordering::Acquire));
        assert_eq!(payload.payload_ref.as_str(), "ref:fs-cli-1");
        assert_eq!(
            sink.descriptors
                .lock()
                .expect("descriptor store")
                .entries
                .get("ref:fs-cli-1")
                .expect("stored descriptor"),
            br#"{"class":"create","paths":["watched/leaf"]}"#
        );
    }

    #[test]
    fn cli_sink_capacity_exhaustion_is_fail_closed() {
        let sink = CliEventSink::new(1, 1024);
        let descriptor = EventDescriptor {
            class: FileEventClass::Create,
            paths: vec!["watched/leaf".to_string()],
        };
        sink.materialize(&descriptor).expect("first descriptor");
        let error = sink
            .materialize(&descriptor)
            .expect_err("second descriptor must exceed event capacity");
        assert!(sink.failed());
        assert_eq!(error.to_string(), "/event_ref_sink: capacity_exhausted");
    }

    #[test]
    fn watch_subjects_are_portable_relative_paths() {
        for accepted in [".", "leaf", "directory/leaf"] {
            validate_watch_subject(accepted).expect(accepted);
        }
        for rejected in ["", "/absolute", "../escape", r"directory\leaf", "C:/drive"] {
            validate_watch_subject(rejected).expect_err(rejected);
        }
    }

    #[test]
    fn uri_shaped_values_are_rejected() {
        for raw in [
            "https://example.invalid/message.json",
            "http://127.0.0.1/message.json",
            "urn:example:waitprims:message",
            "file:/tmp/message.json",
            "file:///tmp/message.json",
            "mailto:ops@example.invalid",
        ] {
            assert!(looks_like_uri(raw), "{raw} must look like a URI");
            reject_non_local_path(Path::new(raw)).expect_err(raw);
        }
        reject_non_local_path(Path::new("-")).expect_err("stdin dash");
    }
}
