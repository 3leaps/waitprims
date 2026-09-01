//! Native local-filesystem [`Observer`] implementation.
//!
//! `waitprims-fs` is a wait arm, not a daemon or shell watch command. It
//! deliberately has no polling fallback and no cross-bind replay claim.

use std::collections::VecDeque;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use notify::event::{ModifyKind, RenameMode};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, Mutex as AsyncMutex};
use waitprims_async::{BindHandle, Observation, Observer};
use waitprims_core::{
    Anchor, AnchorKind, DigestAlgorithm, IdToken, OpaqueRef, PayloadRef, Registration,
    ReplayStatus, Result, Timestamp, ValidationError, WaitEvent,
};

/// Filesystem observer method identifier.
pub const METHOD_FILE_WATCH: &str = "file_watch";
/// Match native create notifications.
pub const PREDICATE_CREATE: &str = "pred:file-create";
/// Match native content/metadata write notifications.
pub const PREDICATE_WRITE: &str = "pred:file-write";
/// Match native remove notifications.
pub const PREDICATE_REMOVE: &str = "pred:file-remove";
/// Match native rename notifications.
pub const PREDICATE_RENAME: &str = "pred:file-rename";
/// Accept every normalized native event, including ambiguous events.
pub const PREDICATE_ANY: &str = "pred:file-any";

const REASON_AMBIGUOUS: &str = "fs_ambiguous_event_class";
const REASON_BIND_RELEASED: &str = "fs_bind_released";
const REASON_CURSOR: &str = "fs_cross_bind_cursor_unsupported";
const REASON_DIGEST: &str = "fs_event_ref_digest_mismatch";
const REASON_INVALID_REGISTRATION: &str = "fs_invalid_registration";
const REASON_NATIVE: &str = "fs_native_watch_failed";
const REASON_PATH: &str = "fs_path_uncertain";
const REASON_RESCAN: &str = "fs_rescan_required";
const REASON_SINK: &str = "fs_event_ref_sink_failed";
const REASON_UNSUPPORTED_FS: &str = "fs_unsupported_filesystem";

static SESSION_SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// Declared posture of the configured watch root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilesystemPosture {
    /// Native local filesystem.
    Local,
    /// Network-backed filesystem. Unsupported in this release.
    Network,
    /// Filesystem posture could not be established. Unsupported in this release.
    Unknown,
}

/// Normalized filesystem event class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileEventClass {
    /// Entry creation.
    Create,
    /// Content or metadata modification.
    Write,
    /// Entry removal.
    Remove,
    /// Entry rename.
    Rename,
    /// Native backend did not provide a portable specific classification.
    Ambiguous,
}

/// Minimal descriptor materialized by a caller-owned event-reference sink.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EventDescriptor {
    /// Normalized event class.
    pub class: FileEventClass,
    /// Sorted, deduplicated, slash-separated paths relative to the watch root.
    pub paths: Vec<String>,
}

impl EventDescriptor {
    /// Stable compact JSON bytes for digesting or storage by a sink.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("EventDescriptor serialization is infallible")
    }
}

/// Caller-owned materialization boundary for event descriptors.
pub trait EventRefSink: Send + Sync {
    /// Store or address `descriptor`, returning a structured payload reference.
    fn materialize(&self, descriptor: &EventDescriptor) -> Result<PayloadRef>;
}

/// Timestamp source injected at the observation boundary.
pub trait EventClock: Send + Sync {
    /// Current contract timestamp.
    fn now(&self) -> Timestamp;
}

/// UTC wall clock used by production observers.
#[derive(Debug, Default)]
pub struct SystemEventClock;

impl EventClock for SystemEventClock {
    fn now(&self) -> Timestamp {
        use time::format_description::well_known::Rfc3339;
        let text = time::OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .expect("UTC timestamp formatting is infallible");
        Timestamp::parse(&text).expect("formatted UTC timestamp satisfies contract profile")
    }
}

/// Native filesystem observer bound to one caller-configured root.
#[derive(Clone)]
pub struct FsObserver {
    source_instance_ref: OpaqueRef,
    root: Arc<PathBuf>,
    posture: FilesystemPosture,
    sink: Arc<dyn EventRefSink>,
    clock: Arc<dyn EventClock>,
    backend: Arc<dyn BackendFactory>,
}

impl FsObserver {
    /// Configure a native observer for one canonical local root.
    pub fn new(
        source_instance_ref: OpaqueRef,
        root: impl AsRef<Path>,
        posture: FilesystemPosture,
        sink: Arc<dyn EventRefSink>,
    ) -> Result<Self> {
        Self::with_clock(
            source_instance_ref,
            root,
            posture,
            sink,
            Arc::new(SystemEventClock),
        )
    }

    /// Configure a native observer with an injected timestamp source.
    pub fn with_clock(
        source_instance_ref: OpaqueRef,
        root: impl AsRef<Path>,
        posture: FilesystemPosture,
        sink: Arc<dyn EventRefSink>,
        clock: Arc<dyn EventClock>,
    ) -> Result<Self> {
        let root = std::fs::canonicalize(root.as_ref()).map_err(|_| {
            ValidationError::new("/filesystem/root", "canonical_local_root_required")
        })?;
        if !root.is_dir() {
            return Err(ValidationError::new("/filesystem/root", "directory_root_required").into());
        }
        Ok(Self {
            source_instance_ref,
            root: Arc::new(root),
            posture,
            sink,
            clock,
            backend: Arc::new(NotifyBackendFactory),
        })
    }

    #[cfg(test)]
    fn with_backend(mut self, backend: Arc<dyn BackendFactory>) -> Self {
        self.backend = backend;
        self
    }

    fn inert_bind(&self, registration: &Registration, terminal: Observation) -> FsBind {
        FsBind::inert(
            registration,
            self.root.clone(),
            self.sink.clone(),
            self.clock.clone(),
            terminal,
        )
    }

    fn validate_registration(
        &self,
        registration: &Registration,
    ) -> std::result::Result<(Predicate, Target), Observation> {
        if registration.method_id.as_str() != METHOD_FILE_WATCH
            || registration.subject_kind.as_str() != "path"
            || registration.source_instance_ref != self.source_instance_ref
        {
            return Err(failed(REASON_INVALID_REGISTRATION));
        }

        if registration.start_anchor.is_some()
            || registration.baseline_policy != Some(waitprims_core::BaselinePolicy::Latest)
        {
            return Err(Observation::CursorUncertain {
                reason_code: IdToken::new(REASON_CURSOR),
            });
        }

        if self.posture != FilesystemPosture::Local {
            return Err(Observation::Degraded {
                reason_code: IdToken::new(REASON_UNSUPPORTED_FS),
            });
        }

        let predicate = Predicate::parse(registration.predicate_ref.as_str())
            .ok_or_else(|| failed(REASON_INVALID_REGISTRATION))?;
        let target = Target::resolve(&self.root, registration.subject_id.as_str())
            .map_err(|_| failed(REASON_PATH))?;
        Ok((predicate, target))
    }
}

impl Observer for FsObserver {
    type Bind = FsBind;

    async fn bind(&self, registration: &Registration) -> Result<Self::Bind> {
        let (predicate, target) = match self.validate_registration(registration) {
            Ok(valid) => valid,
            Err(terminal) => return Ok(self.inert_bind(registration, terminal)),
        };

        let session = SESSION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let resolved_start = anchor(session, 0);
        let capacity = usize::try_from(registration.bounds.max_events)
            .unwrap_or(usize::MAX)
            .clamp(1, 65_536);
        let (sender, receiver) = mpsc::channel(capacity);
        let overflowed = Arc::new(AtomicBool::new(false));
        let callback_overflow = overflowed.clone();
        let callback = Arc::new(move |signal: BackendSignal| {
            if sender.try_send(signal).is_err() {
                callback_overflow.store(true, Ordering::Release);
            }
        });

        let lease = match self.backend.start(&target.watch_paths, callback) {
            Ok(lease) => Some(lease),
            Err(BackendStartError::Failed) => {
                return Ok(self.inert_bind(registration, failed(REASON_NATIVE)))
            }
        };

        Ok(FsBind {
            registration_id: registration.registration_id.clone(),
            registration: registration.clone(),
            resolved_start: resolved_start.clone(),
            root: self.root.clone(),
            target,
            predicate,
            sink: self.sink.clone(),
            clock: self.clock.clone(),
            session,
            sequence: AtomicU64::new(0),
            current_anchor: Mutex::new(resolved_start),
            receiver: AsyncMutex::new(receiver),
            restored: Mutex::new(VecDeque::new()),
            overflowed,
            released: AtomicBool::new(false),
            terminal: Mutex::new(None),
            lease: Mutex::new(lease),
        })
    }

    async fn next(&self, bind: &Self::Bind) -> Result<Observation> {
        loop {
            if let Some(observation) = bind.immediate() {
                return Ok(observation);
            }
            let signal = {
                let mut receiver = bind.receiver.lock().await;
                receiver.recv().await
            };
            match signal {
                Some(signal) => {
                    if let Some(observation) = bind.process(signal) {
                        return Ok(observation);
                    }
                }
                None => return Ok(failed(REASON_NATIVE)),
            }
        }
    }

    async fn cancel(&self, bind: &Self::Bind) -> Result<()> {
        bind.release();
        Ok(())
    }

    fn poll_ready(&self, bind: &Self::Bind) -> Option<Observation> {
        if let Some(observation) = bind.immediate() {
            return Some(observation);
        }
        let mut receiver = bind.receiver.try_lock().ok()?;
        loop {
            match receiver.try_recv() {
                Ok(signal) => {
                    if let Some(observation) = bind.process(signal) {
                        return Some(observation);
                    }
                }
                Err(_) => return None,
            }
        }
    }

    fn restore_ready(&self, bind: &Self::Bind, observation: Observation) -> Result<()> {
        if !bind.owns(&observation) {
            return Err(
                ValidationError::new("/observer/restore_ready", "cross_bind_restore").into(),
            );
        }
        bind.restored
            .lock()
            .expect("filesystem restore queue")
            .push_front(observation);
        Ok(())
    }
}

/// Handle retaining native watch custody for one registration.
pub struct FsBind {
    registration_id: IdToken,
    registration: Registration,
    resolved_start: Anchor,
    root: Arc<PathBuf>,
    target: Target,
    predicate: Predicate,
    sink: Arc<dyn EventRefSink>,
    clock: Arc<dyn EventClock>,
    session: u64,
    sequence: AtomicU64,
    current_anchor: Mutex<Anchor>,
    receiver: AsyncMutex<mpsc::Receiver<BackendSignal>>,
    restored: Mutex<VecDeque<Observation>>,
    overflowed: Arc<AtomicBool>,
    released: AtomicBool,
    terminal: Mutex<Option<Observation>>,
    lease: Mutex<Option<Box<dyn BackendLease>>>,
}

impl FsBind {
    fn owns(&self, observation: &Observation) -> bool {
        let Observation::Event(event) = observation else {
            return false;
        };
        let Some(event_sequence) =
            fs_token_sequence(event.event_id.as_str(), "evt:fs", self.session)
        else {
            return false;
        };

        event.registration_id == self.registration.registration_id
            && event.source_instance_ref == self.registration.source_instance_ref
            && event.method_id == self.registration.method_id
            && event.subject_kind == self.registration.subject_kind
            && event.subject_id == self.registration.subject_id
            && fs_token_sequence(event.correlation_id.as_str(), "cor:fs", self.session)
                == Some(event_sequence)
            && event.start_anchor.kind == AnchorKind::ProviderOpaque
            && fs_token_sequence(event.start_anchor.value.as_str(), "anc:fs", self.session)
                == event_sequence.checked_sub(1)
            && event.proposed_next_anchor.kind == AnchorKind::ProviderOpaque
            && fs_token_sequence(
                event.proposed_next_anchor.value.as_str(),
                "anc:fs",
                self.session,
            ) == Some(event_sequence)
    }

    fn inert(
        registration: &Registration,
        root: Arc<PathBuf>,
        sink: Arc<dyn EventRefSink>,
        clock: Arc<dyn EventClock>,
        terminal: Observation,
    ) -> Self {
        let session = SESSION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let (_sender, receiver) = mpsc::channel(1);
        Self {
            registration_id: registration.registration_id.clone(),
            registration: registration.clone(),
            resolved_start: anchor(session, 0),
            root,
            target: Target::inert(),
            predicate: Predicate::Any,
            sink,
            clock,
            session,
            sequence: AtomicU64::new(0),
            current_anchor: Mutex::new(anchor(session, 0)),
            receiver: AsyncMutex::new(receiver),
            restored: Mutex::new(VecDeque::new()),
            overflowed: Arc::new(AtomicBool::new(false)),
            released: AtomicBool::new(false),
            terminal: Mutex::new(Some(terminal)),
            lease: Mutex::new(None),
        }
    }

    fn immediate(&self) -> Option<Observation> {
        if self.released.load(Ordering::Acquire) {
            return Some(failed(REASON_BIND_RELEASED));
        }
        if let Some(observation) = self
            .restored
            .lock()
            .expect("filesystem restore queue")
            .pop_front()
        {
            return Some(observation);
        }
        if self.overflowed.swap(false, Ordering::AcqRel) {
            return Some(Observation::Overflow);
        }
        self.terminal.lock().expect("filesystem terminal").take()
    }

    fn process(&self, signal: BackendSignal) -> Option<Observation> {
        if self.released.load(Ordering::Acquire) {
            return Some(failed(REASON_BIND_RELEASED));
        }
        match signal {
            BackendSignal::Rescan => Some(Observation::CursorUncertain {
                reason_code: IdToken::new(REASON_RESCAN),
            }),
            BackendSignal::Failed => Some(failed(REASON_NATIVE)),
            BackendSignal::Event(raw) => self.process_event(raw),
        }
    }

    fn process_event(&self, raw: BackendEvent) -> Option<Observation> {
        if !self.target.still_safe(&self.root) {
            return Some(failed(REASON_PATH));
        }

        let paths = match normalize_paths(&self.root, &self.target, raw.paths) {
            Ok(paths) if !paths.is_empty() => paths,
            Ok(_) => return None,
            Err(_) => return Some(failed(REASON_PATH)),
        };

        if raw.class == FileEventClass::Ambiguous && self.predicate != Predicate::Any {
            return Some(Observation::Degraded {
                reason_code: IdToken::new(REASON_AMBIGUOUS),
            });
        }
        if !self.predicate.matches(raw.class) {
            return None;
        }

        let descriptor = EventDescriptor {
            class: raw.class,
            paths,
        };
        if descriptor.canonical_bytes().len() as u64 > self.registration.bounds.max_bytes {
            return Some(Observation::Overflow);
        }
        let payload = match self.sink.materialize(&descriptor) {
            Ok(payload) => payload,
            Err(_) => return Some(failed(REASON_SINK)),
        };
        let expected_digest = hex_sha256(&descriptor.canonical_bytes());
        if payload.content_digest.algorithm != DigestAlgorithm::Sha256
            || payload.content_digest.value != expected_digest
        {
            return Some(failed(REASON_DIGEST));
        }

        let sequence = self.sequence.fetch_add(1, Ordering::Relaxed) + 1;
        let start_anchor = self
            .current_anchor
            .lock()
            .expect("filesystem anchor")
            .clone();
        let proposed_next_anchor = anchor(self.session, sequence);
        *self.current_anchor.lock().expect("filesystem anchor") = proposed_next_anchor.clone();
        let now = self.clock.now();
        Some(Observation::Event(Box::new(WaitEvent {
            event_id: IdToken::new(format!("evt:fs-{}-{sequence}", self.session)),
            registration_id: self.registration.registration_id.clone(),
            source_instance_ref: self.registration.source_instance_ref.clone(),
            method_id: self.registration.method_id.clone(),
            subject_kind: self.registration.subject_kind.clone(),
            subject_id: self.registration.subject_id.clone(),
            occurred_at: now.clone(),
            observed_at: now,
            start_anchor,
            proposed_next_anchor,
            replay_status: ReplayStatus::Fresh,
            correlation_id: IdToken::new(format!("cor:fs-{}-{sequence}", self.session)),
            causation_id: None,
            payload,
            delivery_ref: None,
            activation_ref: None,
        })))
    }

    fn release(&self) {
        if !self.released.swap(true, Ordering::AcqRel) {
            self.lease.lock().expect("filesystem lease").take();
        }
    }
}

impl BindHandle for FsBind {
    fn registration_id(&self) -> &IdToken {
        &self.registration_id
    }

    fn resolved_start(&self) -> &Anchor {
        &self.resolved_start
    }
}

impl Drop for FsBind {
    fn drop(&mut self) {
        self.release();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Predicate {
    Create,
    Write,
    Remove,
    Rename,
    Any,
}

impl Predicate {
    fn parse(value: &str) -> Option<Self> {
        match value {
            PREDICATE_CREATE => Some(Self::Create),
            PREDICATE_WRITE => Some(Self::Write),
            PREDICATE_REMOVE => Some(Self::Remove),
            PREDICATE_RENAME => Some(Self::Rename),
            PREDICATE_ANY => Some(Self::Any),
            _ => None,
        }
    }

    fn matches(self, class: FileEventClass) -> bool {
        self == Self::Any
            || matches!(
                (self, class),
                (Self::Create, FileEventClass::Create)
                    | (Self::Write, FileEventClass::Write)
                    | (Self::Remove, FileEventClass::Remove)
                    | (Self::Rename, FileEventClass::Rename)
            )
    }
}

#[derive(Debug, Clone)]
struct Target {
    relative: PathBuf,
    watch_paths: Vec<PathBuf>,
    canonical_at_bind: Option<PathBuf>,
    canonical_candidate: PathBuf,
    container_logical: PathBuf,
    container_canonical: PathBuf,
    exact_leaf: bool,
}

impl Target {
    fn inert() -> Self {
        Self {
            relative: PathBuf::from("."),
            watch_paths: Vec::new(),
            canonical_at_bind: None,
            canonical_candidate: PathBuf::new(),
            container_logical: PathBuf::new(),
            container_canonical: PathBuf::new(),
            exact_leaf: false,
        }
    }

    fn resolve(root: &Path, subject: &str) -> std::result::Result<Self, ()> {
        let relative = parse_portable_relative(subject)?;
        let logical = root.join(&relative);
        let canonical_at_bind = std::fs::canonicalize(&logical).ok();

        let (watch_paths, canonical_candidate, container_logical, container_canonical, exact_leaf) =
            if let Some(canonical) = &canonical_at_bind {
                if !canonical.starts_with(root) {
                    return Err(());
                }
                if canonical.is_dir() {
                    let logical_parent = logical.parent().unwrap_or(root);
                    let container_logical = if logical == root {
                        root.to_path_buf()
                    } else {
                        logical_parent.to_path_buf()
                    };
                    let container_canonical =
                        std::fs::canonicalize(&container_logical).map_err(|_| ())?;
                    (
                        vec![canonical.clone()],
                        canonical.clone(),
                        container_logical,
                        container_canonical,
                        false,
                    )
                } else {
                    let logical_parent = logical.parent().ok_or(())?.to_path_buf();
                    let canonical_parent =
                        std::fs::canonicalize(&logical_parent).map_err(|_| ())?;
                    (
                        vec![canonical.clone(), canonical_parent.clone()],
                        canonical.clone(),
                        logical_parent,
                        canonical_parent,
                        true,
                    )
                }
            } else {
                let parent = logical.parent().ok_or(())?;
                let canonical_parent = std::fs::canonicalize(parent).map_err(|_| ())?;
                if !canonical_parent.starts_with(root) {
                    return Err(());
                }
                let canonical_candidate = canonical_parent.join(logical.file_name().ok_or(())?);
                (
                    vec![canonical_parent.clone()],
                    canonical_candidate,
                    parent.to_path_buf(),
                    canonical_parent,
                    true,
                )
            };
        if !container_canonical.starts_with(root) {
            return Err(());
        }

        Ok(Self {
            relative,
            watch_paths,
            canonical_at_bind,
            canonical_candidate,
            container_logical,
            container_canonical,
            exact_leaf,
        })
    }

    fn still_safe(&self, root: &Path) -> bool {
        if std::fs::canonicalize(&self.container_logical)
            .ok()
            .is_none_or(|current| current != self.container_canonical)
        {
            return false;
        }
        let logical = root.join(&self.relative);
        match (
            &self.canonical_at_bind,
            std::fs::canonicalize(&logical).ok(),
        ) {
            (Some(expected), Some(current)) => current == *expected && current.starts_with(root),
            (Some(_), None) => true,
            (None, Some(current)) => current.starts_with(root),
            (None, None) => logical
                .parent()
                .and_then(|parent| std::fs::canonicalize(parent).ok())
                .is_some_and(|parent| parent.starts_with(root)),
        }
    }
}

fn parse_portable_relative(subject: &str) -> std::result::Result<PathBuf, ()> {
    if subject.is_empty()
        || subject.starts_with('/')
        || subject.starts_with('\\')
        || subject.contains('\\')
        || subject.as_bytes().get(1) == Some(&b':')
    {
        return Err(());
    }
    let path = Path::new(subject);
    for component in path.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return Err(()),
        }
    }
    Ok(path.to_path_buf())
}

fn normalize_paths(
    root: &Path,
    target: &Target,
    paths: Vec<PathBuf>,
) -> std::result::Result<Vec<String>, ()> {
    let logical_target = root.join(&target.relative);
    let mut normalized = Vec::new();
    for path in paths {
        let absolute = if path.is_absolute() {
            path
        } else {
            root.join(path)
        };
        let safe_absolute = if absolute.exists() {
            std::fs::canonicalize(&absolute).map_err(|_| ())?
        } else {
            let parent = absolute.parent().ok_or(())?;
            let canonical_parent = std::fs::canonicalize(parent).map_err(|_| ())?;
            canonical_parent.join(absolute.file_name().ok_or(())?)
        };
        if !safe_absolute.starts_with(root) {
            return Err(());
        }
        if target.exact_leaf {
            let is_target = absolute == logical_target
                || safe_absolute == logical_target
                || safe_absolute == target.canonical_candidate
                || target
                    .canonical_at_bind
                    .as_ref()
                    .is_some_and(|canonical| *canonical == safe_absolute);
            if !is_target {
                continue;
            }
        }
        let relative = safe_absolute.strip_prefix(root).map_err(|_| ())?;
        normalized.push(portable_path(relative));
    }
    normalized.sort();
    normalized.dedup();
    Ok(normalized)
}

fn portable_path(path: &Path) -> String {
    if path.as_os_str().is_empty() {
        return ".".to_string();
    }
    path.components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn anchor(session: u64, sequence: u64) -> Anchor {
    Anchor {
        kind: AnchorKind::ProviderOpaque,
        value: IdToken::new(format!("anc:fs-{session}-{sequence}")),
    }
}

fn fs_token_sequence(value: &str, namespace: &str, expected_session: u64) -> Option<u64> {
    let remainder = value.strip_prefix(namespace)?.strip_prefix('-')?;
    let (session, sequence) = remainder.split_once('-')?;
    (session.parse::<u64>().ok()? == expected_session)
        .then(|| sequence.parse::<u64>().ok())
        .flatten()
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn failed(reason: &str) -> Observation {
    Observation::Failed {
        reason_code: IdToken::new(reason),
    }
}

#[derive(Debug)]
struct BackendEvent {
    class: FileEventClass,
    paths: Vec<PathBuf>,
}

#[derive(Debug)]
enum BackendSignal {
    Event(BackendEvent),
    Rescan,
    Failed,
}

type BackendCallback = Arc<dyn Fn(BackendSignal) + Send + Sync>;

#[derive(Debug, Clone, Copy)]
enum BackendStartError {
    Failed,
}

trait BackendLease: Send {}

trait BackendFactory: Send + Sync {
    fn start(
        &self,
        paths: &[PathBuf],
        callback: BackendCallback,
    ) -> std::result::Result<Box<dyn BackendLease>, BackendStartError>;
}

struct NotifyBackendFactory;

struct NotifyLease {
    _watcher: RecommendedWatcher,
}

impl BackendLease for NotifyLease {}

impl BackendFactory for NotifyBackendFactory {
    fn start(
        &self,
        paths: &[PathBuf],
        callback: BackendCallback,
    ) -> std::result::Result<Box<dyn BackendLease>, BackendStartError> {
        let event_callback = callback.clone();
        let mut watcher = notify::recommended_watcher(move |result| match result {
            Ok(event) => event_callback(notify_signal(event)),
            Err(_) => event_callback(BackendSignal::Failed),
        })
        .map_err(|_| BackendStartError::Failed)?;
        for path in paths {
            watcher
                .watch(path, RecursiveMode::NonRecursive)
                .map_err(|_| BackendStartError::Failed)?;
        }
        Ok(Box::new(NotifyLease { _watcher: watcher }))
    }
}

fn notify_signal(event: Event) -> BackendSignal {
    if event.need_rescan() {
        return BackendSignal::Rescan;
    }
    let class = match event.kind {
        EventKind::Create(_) => FileEventClass::Create,
        EventKind::Remove(_) => FileEventClass::Remove,
        EventKind::Modify(ModifyKind::Name(
            RenameMode::Any
            | RenameMode::From
            | RenameMode::To
            | RenameMode::Both
            | RenameMode::Other,
        )) => FileEventClass::Rename,
        EventKind::Modify(_) => FileEventClass::Write,
        _ => FileEventClass::Ambiguous,
    };
    BackendSignal::Event(BackendEvent {
        class,
        paths: event.paths,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::fs;
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use tokio::time::timeout;
    use waitprims_async::{
        run_coalesce, run_first_match, run_follow, run_poll_cycle, BindHandle, Cancel,
        CoalescePolicy, FollowEnd, Observation, Observer,
    };
    use waitprims_core::{
        Anchor, AnchorKind, BaselinePolicy, ContentDigest, DigestAlgorithm, IdToken, OpaqueRef,
        OutcomeKind, PayloadRef, PredicateRef, Registration, Timestamp, ValidationError, WaitBound,
    };
    use waitprims_testkit::{live_wait_request, poll_cycle_request, registration_set, FakeClock};

    use super::*;

    static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new() -> Self {
            let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "waitprims-fs-test-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("create temporary root");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[derive(Default)]
    struct RecordingSink {
        descriptors: Mutex<Vec<EventDescriptor>>,
        fail: AtomicBool,
        tamper_digest: AtomicBool,
    }

    impl RecordingSink {
        fn failing() -> Self {
            Self {
                descriptors: Mutex::new(Vec::new()),
                fail: AtomicBool::new(true),
                tamper_digest: AtomicBool::new(false),
            }
        }

        fn tampered() -> Self {
            Self {
                descriptors: Mutex::new(Vec::new()),
                fail: AtomicBool::new(false),
                tamper_digest: AtomicBool::new(true),
            }
        }
    }

    impl EventRefSink for RecordingSink {
        fn materialize(&self, descriptor: &EventDescriptor) -> Result<PayloadRef> {
            if self.fail.load(Ordering::Acquire) {
                return Err(
                    ValidationError::new("/event_ref_sink", "injected_sink_failure").into(),
                );
            }
            self.descriptors
                .lock()
                .expect("recording sink")
                .push(descriptor.clone());
            let digest = if self.tamper_digest.load(Ordering::Acquire) {
                "0".repeat(64)
            } else {
                hex_sha256(&descriptor.canonical_bytes())
            };
            Ok(PayloadRef {
                payload_ref: OpaqueRef::new(format!(
                    "ref:fs-test-{}",
                    self.descriptors.lock().expect("recording sink").len()
                )),
                content_digest: ContentDigest {
                    algorithm: DigestAlgorithm::Sha256,
                    value: digest,
                },
                media_type: Some("application/vnd.waitprims.fs-event+json".to_string()),
            })
        }
    }

    struct FixedClock(Timestamp);

    impl Default for FixedClock {
        fn default() -> Self {
            Self(Timestamp::parse("2026-08-31T20:00:00Z").expect("timestamp"))
        }
    }

    impl EventClock for FixedClock {
        fn now(&self) -> Timestamp {
            self.0.clone()
        }
    }

    #[derive(Default)]
    struct FakeBackend {
        callback: Mutex<Option<BackendCallback>>,
        fail_start: AtomicBool,
        drops: Arc<AtomicUsize>,
    }

    impl FakeBackend {
        fn emit(&self, signal: BackendSignal) {
            self.callback
                .lock()
                .expect("fake callback")
                .as_ref()
                .expect("backend started")(signal);
        }
    }

    struct FakeLease(Arc<AtomicUsize>);

    impl BackendLease for FakeLease {}

    impl Drop for FakeLease {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::AcqRel);
        }
    }

    impl BackendFactory for FakeBackend {
        fn start(
            &self,
            _paths: &[PathBuf],
            callback: BackendCallback,
        ) -> std::result::Result<Box<dyn BackendLease>, BackendStartError> {
            if self.fail_start.load(Ordering::Acquire) {
                return Err(BackendStartError::Failed);
            }
            *self.callback.lock().expect("fake callback") = Some(callback);
            Ok(Box::new(FakeLease(self.drops.clone())))
        }
    }

    fn registration(subject: &str, predicate: &str) -> Registration {
        Registration {
            registration_id: IdToken::new("reg:fs-test"),
            method_id: IdToken::new(METHOD_FILE_WATCH),
            subject_kind: IdToken::new("path"),
            subject_id: IdToken::new(subject),
            required: true,
            source_instance_ref: OpaqueRef::new("src:fs-test"),
            predicate_ref: PredicateRef::new(predicate),
            capability_ref: OpaqueRef::new("cap:fs-test"),
            lease_expires_at: Timestamp::parse("2026-09-01T00:00:00Z").expect("timestamp"),
            bounds: WaitBound {
                max_events: 128,
                max_bytes: 4096,
            },
            start_anchor: None,
            baseline_policy: Some(BaselinePolicy::Latest),
            priority: None,
        }
    }

    fn fake_observer(
        root: &TempRoot,
        sink: Arc<dyn EventRefSink>,
        backend: Arc<FakeBackend>,
    ) -> FsObserver {
        FsObserver::with_clock(
            OpaqueRef::new("src:fs-test"),
            root.path(),
            FilesystemPosture::Local,
            sink,
            Arc::new(FixedClock::default()),
        )
        .expect("observer")
        .with_backend(backend)
    }

    fn event(class: FileEventClass, path: impl Into<PathBuf>) -> BackendSignal {
        BackendSignal::Event(BackendEvent {
            class,
            paths: vec![path.into()],
        })
    }

    fn assert_reason(observation: Observation, expected: &str) {
        let actual = match observation {
            Observation::Failed { reason_code }
            | Observation::Degraded { reason_code }
            | Observation::CursorUncertain { reason_code } => reason_code,
            other => panic!("expected terminal observation, got {other:?}"),
        };
        assert_eq!(actual.as_str(), expected);
    }

    async fn emit_after_bind(backend: Arc<FakeBackend>, class: FileEventClass, path: PathBuf) {
        loop {
            if backend.callback.lock().expect("fake callback").is_some() {
                backend.emit(event(class, path));
                return;
            }
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test]
    async fn explicit_and_non_latest_starts_are_inert_cursor_uncertain_binds() {
        let root = TempRoot::new();
        let backend = Arc::new(FakeBackend::default());
        let observer = fake_observer(&root, Arc::new(RecordingSink::default()), backend.clone());
        let mut explicit = registration(".", PREDICATE_ANY);
        explicit.start_anchor = Some(Anchor {
            kind: AnchorKind::ProviderOpaque,
            value: IdToken::new("anc:prior-session"),
        });
        explicit.baseline_policy = None;

        let bind = observer.bind(&explicit).await.expect("inert bind");
        assert!(bind.resolved_start().value.as_str().starts_with("anc:fs-"));
        assert_reason(observer.next(&bind).await.expect("terminal"), REASON_CURSOR);
        assert!(backend.callback.lock().expect("fake callback").is_none());

        let mut earliest = registration(".", PREDICATE_ANY);
        earliest.baseline_policy = Some(BaselinePolicy::Earliest);
        let bind = observer.bind(&earliest).await.expect("inert bind");
        assert_reason(observer.next(&bind).await.expect("terminal"), REASON_CURSOR);
    }

    #[tokio::test]
    async fn setup_failure_and_unsupported_posture_are_typed_terminals() {
        let root = TempRoot::new();
        let backend = Arc::new(FakeBackend::default());
        backend.fail_start.store(true, Ordering::Release);
        let observer = fake_observer(&root, Arc::new(RecordingSink::default()), backend);
        let bind = observer
            .bind(&registration(".", PREDICATE_ANY))
            .await
            .expect("inert bind");
        assert_reason(observer.next(&bind).await.expect("terminal"), REASON_NATIVE);

        let observer = FsObserver::with_clock(
            OpaqueRef::new("src:fs-test"),
            root.path(),
            FilesystemPosture::Network,
            Arc::new(RecordingSink::default()),
            Arc::new(FixedClock::default()),
        )
        .expect("observer");
        let bind = observer
            .bind(&registration(".", PREDICATE_ANY))
            .await
            .expect("inert bind");
        assert_reason(
            observer.next(&bind).await.expect("terminal"),
            REASON_UNSUPPORTED_FS,
        );
    }

    #[tokio::test]
    async fn same_bind_restore_preserves_fifo_and_anchor_progression() {
        let root = TempRoot::new();
        let watched = root.path().join("watched");
        fs::create_dir(&watched).expect("watched dir");
        let sink = Arc::new(RecordingSink::default());
        let backend = Arc::new(FakeBackend::default());
        let observer = fake_observer(&root, sink, backend.clone());
        let bind = observer
            .bind(&registration("watched", PREDICATE_ANY))
            .await
            .expect("bind");

        backend.emit(event(FileEventClass::Create, watched.join("one")));
        let first = observer.next(&bind).await.expect("first");
        backend.emit(event(FileEventClass::Write, watched.join("two")));
        let second = observer.next(&bind).await.expect("second");

        let (first_event, second_event) = match (&first, &second) {
            (Observation::Event(first), Observation::Event(second)) => (first, second),
            other => panic!("expected two events, got {other:?}"),
        };
        let first_event_id = first_event.event_id.clone();
        let second_event_id = second_event.event_id.clone();
        assert_eq!(
            first_event.proposed_next_anchor, second_event.start_anchor,
            "session anchor advances exactly once per normalized event"
        );

        observer
            .restore_ready(&bind, second)
            .expect("restore newest first");
        observer
            .restore_ready(&bind, first)
            .expect("restore oldest last");
        let replay_first = observer.next(&bind).await.expect("replay first");
        let replay_second = observer.next(&bind).await.expect("replay second");
        assert!(
            matches!(replay_first, Observation::Event(event) if event.event_id == first_event_id)
        );
        assert!(
            matches!(replay_second, Observation::Event(event) if event.event_id == second_event_id)
        );
    }

    #[tokio::test]
    async fn cross_bind_restore_fails_closed() {
        let root = TempRoot::new();
        let watched = root.path().join("watched");
        fs::create_dir(&watched).expect("watched dir");
        let backend = Arc::new(FakeBackend::default());
        let observer = fake_observer(&root, Arc::new(RecordingSink::default()), backend.clone());
        let registration = registration("watched", PREDICATE_ANY);
        let first_bind = observer.bind(&registration).await.expect("first bind");

        backend.emit(event(FileEventClass::Create, watched.join("one")));
        let first_observation = observer.next(&first_bind).await.expect("first event");
        let second_bind = observer.bind(&registration).await.expect("second bind");

        let error = observer
            .restore_ready(&second_bind, first_observation)
            .expect_err("cross-bind restore must fail closed");
        assert_eq!(
            error.to_string(),
            "/observer/restore_ready: cross_bind_restore"
        );
        assert!(
            observer.poll_ready(&second_bind).is_none(),
            "rejected observation must not enter the receiving bind"
        );
    }

    #[test]
    fn fs_session_tokens_do_not_accept_decimal_prefix_collisions() {
        assert_eq!(fs_token_sequence("evt:fs-10-1", "evt:fs", 1), None);
        assert_eq!(fs_token_sequence("evt:fs-10-1", "evt:fs", 10), Some(1));
    }

    #[tokio::test]
    async fn overflow_rescan_ambiguous_and_sink_failure_fail_closed() {
        let root = TempRoot::new();
        let watched = root.path().join("watched");
        fs::create_dir(&watched).expect("watched dir");

        let backend = Arc::new(FakeBackend::default());
        let observer = fake_observer(&root, Arc::new(RecordingSink::default()), backend.clone());
        let mut bounded = registration("watched", PREDICATE_ANY);
        bounded.bounds.max_events = 1;
        let bind = observer.bind(&bounded).await.expect("bind");
        backend.emit(event(FileEventClass::Create, watched.join("one")));
        backend.emit(event(FileEventClass::Create, watched.join("two")));
        assert_eq!(
            observer.next(&bind).await.expect("overflow"),
            Observation::Overflow
        );

        let backend = Arc::new(FakeBackend::default());
        let observer = fake_observer(&root, Arc::new(RecordingSink::default()), backend.clone());
        let bind = observer
            .bind(&registration("watched", PREDICATE_ANY))
            .await
            .expect("bind");
        backend.emit(BackendSignal::Rescan);
        assert_reason(observer.next(&bind).await.expect("rescan"), REASON_RESCAN);

        let backend = Arc::new(FakeBackend::default());
        let observer = fake_observer(&root, Arc::new(RecordingSink::default()), backend.clone());
        let bind = observer
            .bind(&registration("watched", PREDICATE_CREATE))
            .await
            .expect("bind");
        backend.emit(event(FileEventClass::Ambiguous, watched.join("one")));
        assert_reason(
            observer.next(&bind).await.expect("ambiguous"),
            REASON_AMBIGUOUS,
        );

        let backend = Arc::new(FakeBackend::default());
        let observer = fake_observer(&root, Arc::new(RecordingSink::failing()), backend.clone());
        let bind = observer
            .bind(&registration("watched", PREDICATE_ANY))
            .await
            .expect("bind");
        backend.emit(event(FileEventClass::Create, watched.join("one")));
        assert_reason(observer.next(&bind).await.expect("sink"), REASON_SINK);

        let backend = Arc::new(FakeBackend::default());
        let observer = fake_observer(&root, Arc::new(RecordingSink::tampered()), backend.clone());
        let bind = observer
            .bind(&registration("watched", PREDICATE_ANY))
            .await
            .expect("bind");
        backend.emit(event(FileEventClass::Create, watched.join("one")));
        assert_reason(observer.next(&bind).await.expect("digest"), REASON_DIGEST);
    }

    #[tokio::test]
    async fn descriptors_are_relative_minimal_sorted_and_deduplicated() {
        let root = TempRoot::new();
        let watched = root.path().join("watched");
        fs::create_dir(&watched).expect("watched dir");
        let sink = Arc::new(RecordingSink::default());
        let backend = Arc::new(FakeBackend::default());
        let observer = fake_observer(&root, sink.clone(), backend.clone());
        let bind = observer
            .bind(&registration("watched", PREDICATE_ANY))
            .await
            .expect("bind");
        backend.emit(BackendSignal::Event(BackendEvent {
            class: FileEventClass::Create,
            paths: vec![watched.join("b"), watched.join("a"), watched.join("a")],
        }));
        assert!(matches!(
            observer.next(&bind).await.expect("event"),
            Observation::Event(_)
        ));
        let descriptors = sink.descriptors.lock().expect("recording sink");
        assert_eq!(
            descriptors.as_slice(),
            &[EventDescriptor {
                class: FileEventClass::Create,
                paths: vec!["watched/a".to_string(), "watched/b".to_string()],
            }]
        );
        let wire = String::from_utf8(descriptors[0].canonical_bytes()).expect("json");
        assert!(!wire.contains(root.path().to_string_lossy().as_ref()));
        assert!(!wire.contains("cap:"));
        assert!(!wire.contains("credential"));
        assert!(!wire.contains("content"));
    }

    #[tokio::test]
    async fn invalid_paths_and_predicates_fail_before_native_watch() {
        let root = TempRoot::new();
        for subject in ["/absolute", "../escape", r"C:/drive", r"unc\\path"] {
            let backend = Arc::new(FakeBackend::default());
            let observer =
                fake_observer(&root, Arc::new(RecordingSink::default()), backend.clone());
            let bind = observer
                .bind(&registration(subject, PREDICATE_ANY))
                .await
                .expect("inert bind");
            assert_reason(observer.next(&bind).await.expect("path"), REASON_PATH);
            assert!(backend.callback.lock().expect("fake callback").is_none());
        }

        let backend = Arc::new(FakeBackend::default());
        let observer = fake_observer(&root, Arc::new(RecordingSink::default()), backend.clone());
        let bind = observer
            .bind(&registration(".", "pred:inline-glob-*"))
            .await
            .expect("inert bind");
        assert_reason(
            observer.next(&bind).await.expect("predicate"),
            REASON_INVALID_REGISTRATION,
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn post_bind_symlink_exchange_fails_instead_of_retargeting() {
        use std::os::unix::fs::symlink;

        let root = TempRoot::new();
        let first = root.path().join("first");
        let second = root.path().join("second");
        fs::create_dir(&first).expect("first");
        fs::create_dir(&second).expect("second");
        let link = root.path().join("link");
        symlink(&first, &link).expect("link");

        let backend = Arc::new(FakeBackend::default());
        let observer = fake_observer(&root, Arc::new(RecordingSink::default()), backend.clone());
        let bind = observer
            .bind(&registration("link", PREDICATE_ANY))
            .await
            .expect("bind");
        fs::remove_file(&link).expect("remove link");
        symlink(&second, &link).expect("exchange link");
        backend.emit(event(FileEventClass::Create, second.join("event")));
        assert_reason(observer.next(&bind).await.expect("exchange"), REASON_PATH);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn nonexistent_leaf_parent_symlink_exchange_fails_closed() {
        use std::os::unix::fs::symlink;

        let root = TempRoot::new();
        let first = root.path().join("first");
        let second = root.path().join("second");
        fs::create_dir(&first).expect("first");
        fs::create_dir(&second).expect("second");
        let link = root.path().join("link");
        symlink(&first, &link).expect("link");

        let backend = Arc::new(FakeBackend::default());
        let observer = fake_observer(&root, Arc::new(RecordingSink::default()), backend.clone());
        let bind = observer
            .bind(&registration("link/new-file", PREDICATE_ANY))
            .await
            .expect("bind");

        fs::remove_file(&link).expect("remove link");
        symlink(&second, &link).expect("exchange link");
        let retargeted = second.join("new-file");
        fs::write(&retargeted, b"retargeted").expect("retargeted leaf");
        backend.emit(event(FileEventClass::Create, retargeted));
        assert_reason(observer.next(&bind).await.expect("exchange"), REASON_PATH);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn nonexistent_leaf_under_stable_symlink_parent_is_observed() {
        use std::os::unix::fs::symlink;

        let root = TempRoot::new();
        let first = root.path().join("first");
        fs::create_dir(&first).expect("first");
        let link = root.path().join("link");
        symlink(&first, &link).expect("link");

        let sink = Arc::new(RecordingSink::default());
        let backend = Arc::new(FakeBackend::default());
        let observer = fake_observer(&root, sink.clone(), backend.clone());
        let bind = observer
            .bind(&registration("link/new-file", PREDICATE_ANY))
            .await
            .expect("bind");

        let created = first.join("new-file");
        fs::write(&created, b"created").expect("create leaf");
        backend.emit(event(FileEventClass::Create, created));
        assert!(matches!(
            observer.next(&bind).await.expect("event"),
            Observation::Event(_)
        ));
        assert_eq!(
            sink.descriptors.lock().expect("recording sink").as_slice(),
            &[EventDescriptor {
                class: FileEventClass::Create,
                paths: vec!["first/new-file".to_string()],
            }]
        );
    }

    #[tokio::test]
    async fn cancel_and_drop_release_the_native_watch_once() {
        let root = TempRoot::new();
        let backend = Arc::new(FakeBackend::default());
        let observer = fake_observer(&root, Arc::new(RecordingSink::default()), backend.clone());
        let bind = observer
            .bind(&registration(".", PREDICATE_ANY))
            .await
            .expect("bind");
        backend.emit(event(
            FileEventClass::Create,
            root.path().join("queued-before-cancel"),
        ));
        observer.cancel(&bind).await.expect("cancel");
        assert_eq!(backend.drops.load(Ordering::Acquire), 1);
        assert_reason(
            observer.next(&bind).await.expect("post cancel"),
            REASON_BIND_RELEASED,
        );
        drop(bind);
        assert_eq!(backend.drops.load(Ordering::Acquire), 1);

        let bind = observer
            .bind(&registration(".", PREDICATE_ANY))
            .await
            .expect("second bind");
        drop(bind);
        assert_eq!(backend.drops.load(Ordering::Acquire), 2);
    }

    #[tokio::test]
    async fn held_deadline_releases_the_native_watch_without_an_emit() {
        let root = TempRoot::new();
        let backend = Arc::new(FakeBackend::default());
        let observer = fake_observer(&root, Arc::new(RecordingSink::default()), backend.clone());
        let set = registration_set(vec![registration(".", PREDICATE_ANY)]);
        let request = live_wait_request();
        let clock =
            FakeClock::auto(Timestamp::parse("2026-08-15T16:02:00Z").expect("runner timestamp"));
        let cancel = Cancel::new();
        let end = timeout(
            Duration::from_secs(5),
            run_follow(&observer, &clock, &cancel, &set, &request, |_burst| async {
                panic!("deadline-only run must not emit");
                #[allow(unreachable_code)]
                Ok(())
            }),
        )
        .await
        .expect("deadline timeout")
        .expect("follow end");
        assert_eq!(end, FollowEnd::Deadline);
        assert_eq!(backend.drops.load(Ordering::Acquire), 1);
    }

    #[tokio::test]
    async fn existing_first_match_and_poll_cycle_runners_accept_fs_observations_unchanged() {
        let root = TempRoot::new();
        let watched = root.path().join("watched");
        fs::create_dir(&watched).expect("watched dir");

        let backend = Arc::new(FakeBackend::default());
        let observer = fake_observer(&root, Arc::new(RecordingSink::default()), backend.clone());
        let registration = registration("watched", PREDICATE_ANY);
        let set = registration_set(vec![registration]);
        let request = live_wait_request();
        let clock =
            FakeClock::manual(Timestamp::parse("2026-08-15T16:02:00Z").expect("runner timestamp"));
        let cancel = Cancel::new();
        let (outcome, ()) = tokio::join!(
            run_first_match(&set, &request, &observer, &clock, &cancel),
            emit_after_bind(
                backend.clone(),
                FileEventClass::Create,
                watched.join("first")
            )
        );
        let outcome = outcome.expect("first-match outcome");
        assert_eq!(outcome.outcome_kind, OutcomeKind::Events);
        assert_eq!(outcome.events.expect("first-match events").len(), 1);

        let backend = Arc::new(FakeBackend::default());
        let observer = fake_observer(&root, Arc::new(RecordingSink::default()), backend.clone());
        let request = poll_cycle_request(&set);
        let poll_cancel = Cancel::new();
        let (outcome, ()) = tokio::join!(
            run_poll_cycle(&set, &request, &observer, &clock, &poll_cancel),
            emit_after_bind(backend, FileEventClass::Write, watched.join("second"))
        );
        let outcome = outcome.expect("poll-cycle outcome");
        assert_eq!(outcome.outcome_kind, OutcomeKind::Events);
        assert_eq!(outcome.events.len(), 1);
    }

    #[tokio::test]
    async fn follow_and_coalesce_keep_one_native_bind_across_two_emissions() {
        let root = TempRoot::new();
        let watched = root.path().join("watched");
        fs::create_dir(&watched).expect("watched dir");
        let mut registration = registration("watched", PREDICATE_ANY);
        registration.priority = Some(100);
        let set = registration_set(vec![registration]);
        let request = live_wait_request();
        let clock =
            FakeClock::manual(Timestamp::parse("2026-08-15T16:02:00Z").expect("runner timestamp"));

        let backend = Arc::new(FakeBackend::default());
        let observer = fake_observer(&root, Arc::new(RecordingSink::default()), backend.clone());
        let cancel = Cancel::new();
        let sink_calls = Arc::new(AtomicUsize::new(0));
        let follow_backend = backend.clone();
        let follow_cancel = cancel.clone();
        let follow_calls = sink_calls.clone();
        let follow_first = watched.join("follow-one");
        let follow_second = watched.join("follow-two");
        let (end, ()) = tokio::join!(
            run_follow(&observer, &clock, &cancel, &set, &request, move |burst| {
                assert_eq!(burst.events.len(), 1);
                let turn = follow_calls.fetch_add(1, Ordering::AcqRel) + 1;
                if turn == 1 {
                    follow_backend.emit(event(FileEventClass::Write, follow_second.clone()));
                } else {
                    follow_cancel.trigger();
                }
                async { Ok(()) }
            },),
            emit_after_bind(backend.clone(), FileEventClass::Create, follow_first)
        );
        assert_eq!(end.expect("follow end"), FollowEnd::Cancel);
        assert_eq!(sink_calls.load(Ordering::Acquire), 2);
        assert_eq!(backend.drops.load(Ordering::Acquire), 1);

        let backend = Arc::new(FakeBackend::default());
        let observer = fake_observer(&root, Arc::new(RecordingSink::default()), backend.clone());
        let cancel = Cancel::new();
        let sink_calls = Arc::new(AtomicUsize::new(0));
        let coalesce_backend = backend.clone();
        let coalesce_cancel = cancel.clone();
        let coalesce_calls = sink_calls.clone();
        let policy = CoalescePolicy::new(Duration::from_secs(30));
        let coalesce_first = watched.join("coalesce-one");
        let coalesce_second = watched.join("coalesce-two");
        let (end, ()) = tokio::join!(
            run_coalesce(
                &observer,
                &clock,
                &cancel,
                &set,
                &request,
                &policy,
                move |burst| {
                    assert_eq!(burst.events.len(), 1);
                    let turn = coalesce_calls.fetch_add(1, Ordering::AcqRel) + 1;
                    if turn == 1 {
                        coalesce_backend
                            .emit(event(FileEventClass::Write, coalesce_second.clone()));
                    } else {
                        coalesce_cancel.trigger();
                    }
                    async { Ok(()) }
                },
            ),
            emit_after_bind(backend.clone(), FileEventClass::Create, coalesce_first)
        );
        assert_eq!(end.expect("coalesce end"), FollowEnd::Cancel);
        assert_eq!(sink_calls.load(Ordering::Acquire), 2);
        assert_eq!(backend.drops.load(Ordering::Acquire), 1);
    }

    async fn wait_for_native_descriptor(
        observer: &FsObserver,
        bind: &FsBind,
        sink: &RecordingSink,
        expected_class: FileEventClass,
        expected_path: &str,
    ) -> WaitEvent {
        let mut unexpected = VecDeque::new();
        loop {
            let observation = observer.next(bind).await.expect("native observation");
            match observation {
                Observation::Event(event) => {
                    let matched = sink
                        .descriptors
                        .lock()
                        .expect("recording sink")
                        .last()
                        .is_some_and(|descriptor| {
                            descriptor.class == expected_class
                                && descriptor.paths.iter().any(|path| path == expected_path)
                        });
                    if matched {
                        return *event;
                    }
                }
                other => unexpected.push_back(other),
            }
            assert!(
                unexpected.len() < 8,
                "too many unexpected native observations"
            );
        }
    }

    async fn next_native_descriptor(
        observer: &FsObserver,
        bind: &FsBind,
        sink: &RecordingSink,
        expected_class: FileEventClass,
        expected_path: &str,
    ) -> WaitEvent {
        timeout(
            Duration::from_secs(5),
            wait_for_native_descriptor(observer, bind, sink, expected_class, expected_path),
        )
        .await
        .unwrap_or_else(|_| panic!("native {expected_class:?} event timeout for {expected_path}"))
    }

    async fn retry_native_action<F>(
        observer: &FsObserver,
        bind: &FsBind,
        sink: &RecordingSink,
        expected_class: FileEventClass,
        expected_path: &str,
        mut action: F,
    ) -> WaitEvent
    where
        F: FnMut(u64),
    {
        for turn in 0u64..4 {
            action(turn);
            if let Ok(event) = timeout(
                Duration::from_secs(2),
                wait_for_native_descriptor(observer, bind, sink, expected_class, expected_path),
            )
            .await
            {
                return event;
            }
        }
        panic!("native action barrier did not become observable");
    }

    #[tokio::test]
    async fn native_watcher_observes_create_write_remove_and_rename_without_sleeps() {
        let root = TempRoot::new();
        let sink = Arc::new(RecordingSink::default());
        let observer = FsObserver::with_clock(
            OpaqueRef::new("src:fs-test"),
            root.path(),
            FilesystemPosture::Local,
            sink.clone(),
            Arc::new(FixedClock::default()),
        )
        .expect("native observer");

        let create_path = root.path().join("created");
        let create_bind = observer
            .bind(&registration("created", PREDICATE_ANY))
            .await
            .expect("create bind");
        let event = retry_native_action(
            &observer,
            &create_bind,
            &sink,
            FileEventClass::Create,
            "created",
            |turn| {
                let _ = fs::remove_file(&create_path);
                fs::write(&create_path, turn.to_le_bytes()).expect("create file");
            },
        )
        .await;
        assert_eq!(event.subject_id.as_str(), "created");
        observer.cancel(&create_bind).await.expect("cancel create");

        let write_path = root.path().join("written");
        fs::write(&write_path, b"before").expect("seed write");
        let write_bind = observer
            .bind(&registration("written", PREDICATE_ANY))
            .await
            .expect("write bind");
        let event = retry_native_action(
            &observer,
            &write_bind,
            &sink,
            FileEventClass::Write,
            "written",
            |turn| fs::write(&write_path, turn.to_le_bytes()).expect("write file"),
        )
        .await;
        assert_eq!(event.subject_id.as_str(), "written");
        observer.cancel(&write_bind).await.expect("cancel write");

        let remove_path = root.path().join("removed");
        fs::write(&remove_path, b"remove").expect("seed remove");
        let remove_bind = observer
            .bind(&registration("removed", PREDICATE_ANY))
            .await
            .expect("remove bind");
        let _ = retry_native_action(
            &observer,
            &remove_bind,
            &sink,
            FileEventClass::Write,
            "removed",
            |turn| fs::write(&remove_path, turn.to_le_bytes()).expect("remove barrier"),
        )
        .await;
        fs::remove_file(&remove_path).expect("remove file");
        let event = next_native_descriptor(
            &observer,
            &remove_bind,
            &sink,
            FileEventClass::Remove,
            "removed",
        )
        .await;
        assert_eq!(event.subject_id.as_str(), "removed");
        observer.cancel(&remove_bind).await.expect("cancel remove");

        let rename_from = root.path().join("rename-from");
        let rename_to = root.path().join("rename-to");
        fs::write(&rename_from, b"rename").expect("seed rename");
        let rename_bind = observer
            .bind(&registration("rename-from", PREDICATE_ANY))
            .await
            .expect("rename bind");
        let _ = retry_native_action(
            &observer,
            &rename_bind,
            &sink,
            FileEventClass::Write,
            "rename-from",
            |turn| fs::write(&rename_from, turn.to_le_bytes()).expect("rename barrier"),
        )
        .await;
        fs::rename(&rename_from, &rename_to).expect("rename file");
        let event = next_native_descriptor(
            &observer,
            &rename_bind,
            &sink,
            FileEventClass::Rename,
            "rename-from",
        )
        .await;
        assert_eq!(event.subject_id.as_str(), "rename-from");
        observer.cancel(&rename_bind).await.expect("cancel rename");

        let descriptors = sink.descriptors.lock().expect("recording sink");
        assert!(descriptors
            .iter()
            .any(|descriptor| descriptor.class == FileEventClass::Create));
        assert!(descriptors
            .iter()
            .any(|descriptor| descriptor.class == FileEventClass::Write));
        assert!(descriptors
            .iter()
            .any(|descriptor| descriptor.class == FileEventClass::Remove));
        assert!(descriptors
            .iter()
            .any(|descriptor| descriptor.class == FileEventClass::Rename));
    }
}
