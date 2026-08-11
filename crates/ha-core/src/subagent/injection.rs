use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use super::helpers::{
    emit_parent_stream_event, release_injection_owner, truncate_str, CleanupGuard,
};
use super::types::{ParentAgentStreamEvent, SubagentStatus};
use super::{
    ActiveInjection, ACTIVE_CHAT_SESSIONS, FETCHED_RUN_IDS, INJECTING_SESSIONS, INJECTION_CANCELS,
    PENDING_INJECTIONS, SESSION_IDLE_NOTIFY,
};

type InjectionReceiptStep = Arc<dyn Fn() -> anyhow::Result<()> + Send + Sync>;

/// Durable source receipt carried with an injection and all of its in-process
/// retries.
///
/// An attached IM mirror can perform a persistent provider mutation before the
/// parent turn settles. It therefore calls `arm_no_replay` before starting the
/// engine. Sources that require a cross-process delivery claim even without an
/// IM mirror (currently persisted wakeups) opt into the same boundary through
/// `pre_engine_claim`. That write must make startup recovery skip this source;
/// a confirmed cancellation may still retry from [`PENDING_INJECTIONS`] in
/// this process, but a crash deliberately loses that automatic retry rather
/// than duplicate provider/tool side effects. `settle` records an ordinary
/// terminal landing and must be idempotent because fetched/cancel races can
/// converge on the same source.
#[derive(Clone)]
pub struct OnInjected {
    arm_no_replay: InjectionReceiptStep,
    settle: InjectionReceiptStep,
    release_unarmed: Option<InjectionReceiptStep>,
    /// Optional process-local dispatch claim retained across FIFO deferral.
    ///
    /// Async-job replay uses this to keep every source job in an in-flight set
    /// while a merged ParentInjection is queued. Release is deliberately tied
    /// to a successful durable arm/settle, or to an explicit pre-arm abandon;
    /// a failed terminal DB write must not reopen the five-second sweep and
    /// immediately duplicate a turn that already reached the GUI.
    release_process_dispatch: Option<Arc<dyn Fn() + Send + Sync>>,
    durable_handoff: bool,
    pre_engine_claim: bool,
    retain_process_dispatch_until_settle: bool,
    no_replay_armed: Arc<AtomicBool>,
    arm_lock: Arc<std::sync::Mutex<()>>,
    unarmed_released: Arc<AtomicBool>,
    process_dispatch_released: Arc<AtomicBool>,
}

impl OnInjected {
    /// Construct a receipt without assuming that the five-second Primary sweep
    /// can rediscover it. Only source families explicitly covered by that sweep
    /// may opt in through [`Self::with_primary_handoff`].
    pub(crate) fn new(
        arm_no_replay: impl Fn() -> anyhow::Result<()> + Send + Sync + 'static,
        settle: impl Fn() -> anyhow::Result<()> + Send + Sync + 'static,
    ) -> Self {
        Self {
            arm_no_replay: Arc::new(arm_no_replay),
            settle: Arc::new(settle),
            release_unarmed: None,
            release_process_dispatch: None,
            durable_handoff: false,
            pre_engine_claim: false,
            retain_process_dispatch_until_settle: false,
            no_replay_armed: Arc::new(AtomicBool::new(false)),
            arm_lock: Arc::new(std::sync::Mutex::new(())),
            unarmed_released: Arc::new(AtomicBool::new(false)),
            process_dispatch_released: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Add an idempotent rollback for a source claim that is abandoned before
    /// its no-replay fence is armed. Durable implementations must predicate the
    /// write on their ordinary in-flight state and never release an armed one.
    pub(crate) fn with_release_unarmed(
        mut self,
        release: impl Fn() -> anyhow::Result<()> + Send + Sync + 'static,
    ) -> Self {
        self.release_unarmed = Some(Arc::new(release));
        self
    }

    /// Retain a process-local source claim for this receipt's complete queued
    /// lifecycle. The callback must be idempotent and non-blocking: it is also
    /// called synchronously when session cleanup purges a queued task.
    pub(crate) fn with_process_dispatch_release(
        mut self,
        release: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        self.release_process_dispatch = Some(Arc::new(release));
        self
    }

    /// Declare that this source is rediscoverable by the periodic Primary
    /// ParentInjection sweep (currently ordinary subagent results and
    /// async_jobs only). Keep this explicit: a durable workflow/wakeup row that
    /// is replayed only at startup is not a live Secondary→Primary handoff.
    pub(crate) fn with_primary_handoff(mut self) -> Self {
        self.durable_handoff = true;
        self
    }

    /// Require this source's durable CAS before every parent engine attempt,
    /// even when no IM mirror is attached. This is for source identities that
    /// may be replayed by another process while the old owner is settling.
    pub(crate) fn with_pre_engine_claim(mut self) -> Self {
        self.pre_engine_claim = true;
        self
    }

    /// Keep the process-local source visible after its no-replay CAS succeeds,
    /// releasing it only when the parent attempt settles or abandons before
    /// arming. This lets owner-side Stop watchers fence an already-claimed
    /// source that another process can no longer enumerate from pending rows.
    pub(crate) fn retain_process_dispatch_until_settle(mut self) -> Self {
        self.retain_process_dispatch_until_settle = true;
        self
    }

    /// Use one idempotent process-local step for both phases. This does not
    /// advertise a durable cross-process replay source; a
    /// Secondary must preserve the callback by running GUI-only locally rather
    /// than abandoning it for a Primary that cannot rediscover it.
    pub(crate) fn idempotent(
        step: impl Fn() -> anyhow::Result<()> + Send + Sync + 'static,
    ) -> Self {
        let step: InjectionReceiptStep = Arc::new(step);
        Self {
            arm_no_replay: step.clone(),
            settle: step,
            release_unarmed: None,
            release_process_dispatch: None,
            durable_handoff: false,
            pre_engine_claim: false,
            retain_process_dispatch_until_settle: false,
            no_replay_armed: Arc::new(AtomicBool::new(false)),
            arm_lock: Arc::new(std::sync::Mutex::new(())),
            unarmed_released: Arc::new(AtomicBool::new(false)),
            process_dispatch_released: Arc::new(AtomicBool::new(false)),
        }
    }

    fn arm_no_replay(&self) -> anyhow::Result<()> {
        if self.no_replay_armed.load(Ordering::Acquire) {
            return Ok(());
        }
        let _arm = self
            .arm_lock
            .lock()
            .map_err(|_| anyhow::anyhow!("injection no-replay arm lock poisoned"))?;
        // A late installer and the core engine boundary may race this method.
        // Re-check under the shared clone-stable lock so the durable CAS/callback
        // runs once; failures leave the flag false and remain retryable.
        if self.no_replay_armed.load(Ordering::Acquire) {
            return Ok(());
        }
        (self.arm_no_replay)()?;
        self.no_replay_armed.store(true, Ordering::Release);
        if !self.retain_process_dispatch_until_settle {
            self.release_process_dispatch();
        }
        Ok(())
    }

    fn is_no_replay_armed(&self) -> bool {
        self.no_replay_armed.load(Ordering::Acquire)
    }

    fn supports_durable_handoff(&self) -> bool {
        self.durable_handoff
    }

    fn requires_pre_engine_claim(&self) -> bool {
        self.pre_engine_claim
    }

    fn supports_unarmed_retry(&self) -> bool {
        self.release_unarmed.is_some()
    }

    fn settle(&self) -> anyhow::Result<()> {
        (self.settle)()?;
        self.release_process_dispatch();
        Ok(())
    }

    fn release_unarmed(&self) -> anyhow::Result<()> {
        self.release_process_dispatch();
        if self.is_no_replay_armed() || self.release_unarmed.is_none() {
            return Ok(());
        }
        if self
            .unarmed_released
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Ok(());
        }
        let result = (self.release_unarmed.as_ref().expect("checked above"))();
        if result.is_err() {
            self.unarmed_released.store(false, Ordering::Release);
        }
        result
    }

    fn release_process_dispatch(&self) {
        let Some(release) = self.release_process_dispatch.as_ref() else {
            return;
        };
        if self
            .process_dispatch_released
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            release();
        }
    }
}

#[derive(Clone)]
enum ActiveInjectionMirrorTerminal {
    Finalize(String),
    Abort(Option<String>),
}

enum ActiveInjectionMirrorPhase {
    /// The injection owner has not resolved its initial IM attach yet. A late
    /// installer may not overtake this phase.
    Initializing,
    /// A binding change requested a late install while the initial channel hook
    /// was still resolving. The reservation waits; an initial `Absent` hands it
    /// the slot, while an initial `Attached` is published for the late token to
    /// retain or replace according to the attach-generation claim.
    LateRequested,
    /// The initial attach was `Absent`; a same-process late attach may reserve
    /// the slot once the ParentInjection stream is visible.
    Open,
    Installing,
    Installed(Box<dyn crate::channel_hooks::ImLiveMirror>),
    /// The initial hook returned an owner after a late reservation. Keep it
    /// quarantined until that reservation either commits its newer generation
    /// or drops without winning the claim.
    LateReplacing(Box<dyn crate::channel_hooks::ImLiveMirror>),
    TerminalPendingReplacement {
        terminal: ActiveInjectionMirrorTerminal,
        previous: Box<dyn crate::channel_hooks::ImLiveMirror>,
    },
    TerminalPendingInstall(ActiveInjectionMirrorTerminal),
    /// A rebind installer disappeared after terminal ownership moved to it.
    /// The old mirror must only be retired; the caller then reattaches the
    /// current durable binding instead of delivering to that stale target.
    RetiringForBackstop(Box<dyn crate::channel_hooks::ImLiveMirror>),
    Completing,
    Completed(crate::channel_hooks::ImLiveMirrorAbortStatus),
    Closed,
}

/// Per-attempt initial/late ParentInjection IM mirror handoff.
///
/// The global active-injection map owns one `Arc`; a late-install reservation
/// owns another. Consequently terminal cleanup may remove the map entry without
/// losing a concurrently arming installer. No provider future runs under this
/// mutex.
pub(crate) struct ActiveInjectionMirrorCoordinator {
    phase: std::sync::Mutex<ActiveInjectionMirrorPhase>,
    terminal_notify: tokio::sync::Notify,
    /// Late binding changes that observed an installer/terminal already in
    /// flight. Registration and phase transitions are serialized by `phase`;
    /// the atomic lets terminal completion inspect the count without adding a
    /// second lock-order edge.
    late_retry_waiters: AtomicUsize,
    receipt: Option<OnInjected>,
}

impl ActiveInjectionMirrorCoordinator {
    pub(crate) fn new(receipt: Option<OnInjected>) -> Self {
        Self {
            phase: std::sync::Mutex::new(ActiveInjectionMirrorPhase::Initializing),
            terminal_notify: tokio::sync::Notify::new(),
            late_retry_waiters: AtomicUsize::new(0),
            receipt,
        }
    }

    fn resolve_initial(&self, mirror: Option<Box<dyn crate::channel_hooks::ImLiveMirror>>) -> bool {
        let Ok(mut phase) = self.phase.lock() else {
            return false;
        };
        *phase = match (
            std::mem::replace(&mut *phase, ActiveInjectionMirrorPhase::Closed),
            mirror,
        ) {
            (ActiveInjectionMirrorPhase::Initializing, Some(mirror)) => {
                ActiveInjectionMirrorPhase::Installed(mirror)
            }
            (ActiveInjectionMirrorPhase::LateRequested, Some(mirror)) => {
                ActiveInjectionMirrorPhase::LateReplacing(mirror)
            }
            (ActiveInjectionMirrorPhase::Initializing, None) => ActiveInjectionMirrorPhase::Open,
            (ActiveInjectionMirrorPhase::LateRequested, None) => {
                ActiveInjectionMirrorPhase::Installing
            }
            (previous, mirror) => {
                drop(mirror);
                *phase = previous;
                return false;
            }
        };
        self.terminal_notify.notify_waiters();
        true
    }

    fn close_initial(&self) {
        let mut changed = false;
        if let Ok(mut phase) = self.phase.lock() {
            if matches!(
                *phase,
                ActiveInjectionMirrorPhase::Initializing
                    | ActiveInjectionMirrorPhase::LateRequested
                    | ActiveInjectionMirrorPhase::Open
            ) {
                *phase = ActiveInjectionMirrorPhase::Closed;
                changed = true;
            }
        }
        if changed {
            self.terminal_notify.notify_waiters();
        }
    }

    fn reserve_late(self: &Arc<Self>) -> LateInjectionMirrorReservation {
        let Ok(mut phase) = self.phase.lock() else {
            return LateInjectionMirrorReservation::Stale;
        };
        match std::mem::replace(&mut *phase, ActiveInjectionMirrorPhase::Closed) {
            ActiveInjectionMirrorPhase::Initializing => {
                *phase = ActiveInjectionMirrorPhase::LateRequested;
                LateInjectionMirrorReservation::Reserved(LateInjectionMirrorInstall {
                    coordinator: self.clone(),
                    previous: None,
                    claimed_binding: false,
                    finished: false,
                })
            }
            ActiveInjectionMirrorPhase::Open => {
                *phase = ActiveInjectionMirrorPhase::Installing;
                LateInjectionMirrorReservation::Reserved(LateInjectionMirrorInstall {
                    coordinator: self.clone(),
                    previous: None,
                    claimed_binding: false,
                    finished: false,
                })
            }
            ActiveInjectionMirrorPhase::Installed(previous) => {
                // Rebind handoff: temporarily take the old owner. If the new
                // attach proves identical (`try_claim` => Busy), token Drop
                // restores it; otherwise the installer aborts it before the new
                // target becomes the sole terminal owner.
                *phase = ActiveInjectionMirrorPhase::Installing;
                LateInjectionMirrorReservation::Reserved(LateInjectionMirrorInstall {
                    coordinator: self.clone(),
                    previous: Some(previous),
                    claimed_binding: false,
                    finished: false,
                })
            }
            previous @ (ActiveInjectionMirrorPhase::LateRequested
            | ActiveInjectionMirrorPhase::Installing
            | ActiveInjectionMirrorPhase::LateReplacing(_)
            | ActiveInjectionMirrorPhase::TerminalPendingReplacement { .. }
            | ActiveInjectionMirrorPhase::RetiringForBackstop(_)
            | ActiveInjectionMirrorPhase::Completing
            | ActiveInjectionMirrorPhase::TerminalPendingInstall(_)) => {
                *phase = previous;
                self.late_retry_waiters.fetch_add(1, Ordering::AcqRel);
                LateInjectionMirrorReservation::Busy(LateInjectionMirrorRetry {
                    coordinator: self.clone(),
                    registered: true,
                })
            }
            previous @ (ActiveInjectionMirrorPhase::Completed(_)
            | ActiveInjectionMirrorPhase::Closed) => {
                *phase = previous;
                LateInjectionMirrorReservation::Stale
            }
        }
    }
}

/// Closes an unresolved initial attach when the parent-injection attempt exits
/// before the channel hook can publish `Attached` or `Absent`.
///
/// The coordinator is visible to binding-change callbacks as soon as it enters
/// `INJECTION_CANCELS`. A callback can therefore reserve the late slot while an
/// earlier validation step is still running. Every exit after registration
/// must wake that reservation; once the initial result is accepted, however,
/// the resulting `Open`/`Installed` owner must remain available until terminal
/// delivery.
struct InitialInjectionMirrorResolutionGuard {
    coordinator: Arc<ActiveInjectionMirrorCoordinator>,
    pending: bool,
}

impl InitialInjectionMirrorResolutionGuard {
    fn new(coordinator: Arc<ActiveInjectionMirrorCoordinator>) -> Self {
        Self {
            coordinator,
            pending: true,
        }
    }

    fn resolve(&mut self, mirror: Option<Box<dyn crate::channel_hooks::ImLiveMirror>>) -> bool {
        let accepted = self.coordinator.resolve_initial(mirror);
        if accepted {
            self.pending = false;
        }
        accepted
    }

    fn close(&mut self) {
        if self.pending {
            self.coordinator.close_initial();
            self.pending = false;
        }
    }
}

impl Drop for InitialInjectionMirrorResolutionGuard {
    fn drop(&mut self) {
        self.close();
    }
}

/// Result of reserving the only late-mirror slot for an active injection.
pub enum LateInjectionMirrorReservation {
    Reserved(LateInjectionMirrorInstall),
    /// Another late install or terminal mutation currently owns the slot.
    /// Awaiting this token atomically converts the registered retry into the
    /// next reservation, so terminal handling cannot slip through the gap and
    /// finalize an intermediate attach.
    Busy(LateInjectionMirrorRetry),
    Stale,
}

/// Registered retry for a binding change that arrived while another late
/// mirror transition was in flight.
pub struct LateInjectionMirrorRetry {
    coordinator: Arc<ActiveInjectionMirrorCoordinator>,
    registered: bool,
}

impl LateInjectionMirrorRetry {
    pub async fn wait(mut self) -> LateInjectionMirrorReservation {
        loop {
            let coordinator = self.coordinator.clone();
            let notified = coordinator.terminal_notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let outcome = {
                let Ok(mut phase) = coordinator.phase.lock() else {
                    self.unregister();
                    return LateInjectionMirrorReservation::Stale;
                };
                match std::mem::replace(&mut *phase, ActiveInjectionMirrorPhase::Closed) {
                    ActiveInjectionMirrorPhase::Initializing => {
                        *phase = ActiveInjectionMirrorPhase::LateRequested;
                        self.unregister_locked();
                        Some(LateInjectionMirrorReservation::Reserved(
                            LateInjectionMirrorInstall {
                                coordinator: coordinator.clone(),
                                previous: None,
                                claimed_binding: false,
                                finished: false,
                            },
                        ))
                    }
                    ActiveInjectionMirrorPhase::Open => {
                        *phase = ActiveInjectionMirrorPhase::Installing;
                        self.unregister_locked();
                        Some(LateInjectionMirrorReservation::Reserved(
                            LateInjectionMirrorInstall {
                                coordinator: coordinator.clone(),
                                previous: None,
                                claimed_binding: false,
                                finished: false,
                            },
                        ))
                    }
                    ActiveInjectionMirrorPhase::Installed(previous) => {
                        *phase = ActiveInjectionMirrorPhase::Installing;
                        self.unregister_locked();
                        Some(LateInjectionMirrorReservation::Reserved(
                            LateInjectionMirrorInstall {
                                coordinator: coordinator.clone(),
                                previous: Some(previous),
                                claimed_binding: false,
                                finished: false,
                            },
                        ))
                    }
                    previous @ (ActiveInjectionMirrorPhase::LateRequested
                    | ActiveInjectionMirrorPhase::Installing
                    | ActiveInjectionMirrorPhase::LateReplacing(_)
                    | ActiveInjectionMirrorPhase::TerminalPendingReplacement {
                        ..
                    }
                    | ActiveInjectionMirrorPhase::TerminalPendingInstall(_)
                    | ActiveInjectionMirrorPhase::RetiringForBackstop(_)
                    | ActiveInjectionMirrorPhase::Completing) => {
                        *phase = previous;
                        None
                    }
                    previous @ (ActiveInjectionMirrorPhase::Completed(_)
                    | ActiveInjectionMirrorPhase::Closed) => {
                        *phase = previous;
                        self.unregister_locked();
                        Some(LateInjectionMirrorReservation::Stale)
                    }
                }
            };
            if let Some(outcome) = outcome {
                self.coordinator.terminal_notify.notify_waiters();
                return outcome;
            }
            notified.as_mut().await;
        }
    }

    fn unregister_locked(&mut self) {
        if self.registered {
            self.coordinator
                .late_retry_waiters
                .fetch_sub(1, Ordering::AcqRel);
            self.registered = false;
        }
    }

    fn unregister(&mut self) {
        let coordinator = self.coordinator.clone();
        if let Ok(_phase) = coordinator.phase.lock() {
            self.unregister_locked();
        }
        coordinator.terminal_notify.notify_waiters();
    }
}

impl Drop for LateInjectionMirrorRetry {
    fn drop(&mut self) {
        self.unregister();
    }
}

/// Reservation held while ha-channel validates the binding, arms the durable
/// receipt and constructs a provider pipeline.
pub struct LateInjectionMirrorInstall {
    coordinator: Arc<ActiveInjectionMirrorCoordinator>,
    previous: Option<Box<dyn crate::channel_hooks::ImLiveMirror>>,
    /// Set only after ha-channel successfully claims the replacement attach
    /// generation. From that point on Drop must never restore an older target.
    claimed_binding: bool,
    finished: bool,
}

impl LateInjectionMirrorInstall {
    /// Close an old attach owner before installing a rebind replacement. The
    /// abort synchronously detaches its sink; awaiting the returned future
    /// settles any already-visible provider preview before the new target can
    /// become terminal owner.
    pub async fn retire_previous(&mut self) -> bool {
        self.claimed_binding = true;
        loop {
            let notified = self.coordinator.terminal_notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let ready = match self.coordinator.phase.lock() {
                Ok(mut phase) => {
                    match std::mem::replace(&mut *phase, ActiveInjectionMirrorPhase::Closed) {
                        ActiveInjectionMirrorPhase::LateRequested => {
                            *phase = ActiveInjectionMirrorPhase::LateRequested;
                            None
                        }
                        ActiveInjectionMirrorPhase::Installed(previous) => {
                            *phase = ActiveInjectionMirrorPhase::Installing;
                            self.previous = Some(previous);
                            Some(true)
                        }
                        ActiveInjectionMirrorPhase::LateReplacing(previous) => {
                            *phase = ActiveInjectionMirrorPhase::Installing;
                            self.previous = Some(previous);
                            Some(true)
                        }
                        ActiveInjectionMirrorPhase::TerminalPendingReplacement {
                            terminal,
                            previous,
                        } => {
                            *phase = ActiveInjectionMirrorPhase::TerminalPendingInstall(terminal);
                            self.previous = Some(previous);
                            Some(true)
                        }
                        previous @ (ActiveInjectionMirrorPhase::Installing
                        | ActiveInjectionMirrorPhase::TerminalPendingInstall(_)) => {
                            *phase = previous;
                            Some(true)
                        }
                        previous => {
                            *phase = previous;
                            Some(false)
                        }
                    }
                }
                Err(_) => Some(false),
            };
            match ready {
                Some(ready) => {
                    if !ready {
                        return false;
                    }
                    break;
                }
                None => notified.as_mut().await,
            }
        }

        let Some(previous) = self.previous.take() else {
            return true;
        };
        let status = previous.abort(None).await;
        if status.is_confirmed() {
            return true;
        }
        self.finished = true;
        if let Ok(mut phase) = self.coordinator.phase.lock() {
            *phase = ActiveInjectionMirrorPhase::Completed(status);
        }
        self.coordinator.terminal_notify.notify_waiters();
        false
    }

    /// Persist the same no-replay fence used by the initial IM attach. This is
    /// mandatory before a late pipeline can perform its first provider write.
    pub async fn arm_no_replay(&mut self, run_id: &str) -> bool {
        // This also atomically takes over an initial mirror that resolved after
        // the late reservation. The replacement attach has already won its
        // generation claim, so the stale owner must be detached first.
        if !self.retire_previous().await {
            return false;
        }
        arm_injection_mirror_receipt(&self.coordinator, run_id).await
    }

    /// Install a fully constructed late mirror. If the engine reached terminal
    /// state while the provider pipeline was being prepared, this method owns
    /// that terminal and completes it before returning.
    pub async fn install(mut self, mirror: Box<dyn crate::channel_hooks::ImLiveMirror>) -> bool {
        if !self.retire_previous().await {
            self.finished = true;
            let _ = mirror.abort(None).await;
            if let Ok(mut phase) = self.coordinator.phase.lock() {
                *phase = ActiveInjectionMirrorPhase::Completed(
                    crate::channel_hooks::ImLiveMirrorAbortStatus::Unsafe,
                );
            }
            self.coordinator.terminal_notify.notify_waiters();
            return false;
        }
        enum InstallAction {
            Stored,
            Complete(
                ActiveInjectionMirrorTerminal,
                Box<dyn crate::channel_hooks::ImLiveMirror>,
            ),
            Reject(Box<dyn crate::channel_hooks::ImLiveMirror>),
        }
        let action = match self.coordinator.phase.lock() {
            Ok(mut phase) => {
                match std::mem::replace(&mut *phase, ActiveInjectionMirrorPhase::Closed) {
                    ActiveInjectionMirrorPhase::Installing => {
                        *phase = ActiveInjectionMirrorPhase::Installed(mirror);
                        InstallAction::Stored
                    }
                    ActiveInjectionMirrorPhase::TerminalPendingInstall(terminal) => {
                        *phase = ActiveInjectionMirrorPhase::Completing;
                        InstallAction::Complete(terminal, mirror)
                    }
                    previous => {
                        *phase = previous;
                        InstallAction::Reject(mirror)
                    }
                }
            }
            Err(_) => InstallAction::Reject(mirror),
        };

        self.finished = true;
        let (terminal, mirror) = match action {
            InstallAction::Stored => {
                // Wake a binding change that arrived while this pipeline was
                // being built. Its retry remains registered until it
                // atomically takes the next slot.
                self.coordinator.terminal_notify.notify_waiters();
                return true;
            }
            InstallAction::Complete(terminal, mirror) => (terminal, mirror),
            InstallAction::Reject(mirror) => {
                let _ = mirror.abort(None).await;
                return false;
            }
        };
        let status = apply_injection_mirror_terminal(mirror, terminal).await;
        if let Ok(mut phase) = self.coordinator.phase.lock() {
            *phase = if self.coordinator.late_retry_waiters.load(Ordering::Acquire) > 0 {
                // A newer binding arrived while this terminal future was in
                // flight. Provider-side guards make the old target a no-op
                // after the DB rebind; reopen so the same core terminal lands
                // on the registered latest target.
                ActiveInjectionMirrorPhase::Open
            } else {
                ActiveInjectionMirrorPhase::Completed(status)
            };
        }
        self.coordinator.terminal_notify.notify_waiters();
        true
    }
}

async fn arm_injection_mirror_receipt(
    coordinator: &ActiveInjectionMirrorCoordinator,
    run_id: &str,
) -> bool {
    let Some(receipt) = coordinator.receipt.clone() else {
        // Process-local sources have no durable restart path.
        return true;
    };
    let arm_run_id = run_id.to_string();
    let result = crate::blocking::run_blocking(move || receipt.arm_no_replay()).await;
    if let Err(error) = result {
        app_error!(
            "subagent",
            "inject::late_im_arm",
            "Failed to arm late IM delivery for run {}: {}",
            arm_run_id,
            crate::logging::redact_sensitive(&error.to_string())
        );
        return false;
    }
    true
}

impl Drop for LateInjectionMirrorInstall {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        let notify = if let Ok(mut phase) = self.coordinator.phase.lock() {
            *phase = match std::mem::replace(&mut *phase, ActiveInjectionMirrorPhase::Closed) {
                ActiveInjectionMirrorPhase::LateRequested if !self.claimed_binding => {
                    ActiveInjectionMirrorPhase::Initializing
                }
                ActiveInjectionMirrorPhase::LateRequested => ActiveInjectionMirrorPhase::Closed,
                ActiveInjectionMirrorPhase::LateReplacing(previous) if !self.claimed_binding => {
                    ActiveInjectionMirrorPhase::Installed(previous)
                }
                ActiveInjectionMirrorPhase::LateReplacing(previous) => {
                    ActiveInjectionMirrorPhase::RetiringForBackstop(previous)
                }
                ActiveInjectionMirrorPhase::TerminalPendingReplacement {
                    terminal: _,
                    previous,
                } if !self.claimed_binding => ActiveInjectionMirrorPhase::Installed(previous),
                ActiveInjectionMirrorPhase::TerminalPendingReplacement {
                    terminal: _,
                    previous,
                } => ActiveInjectionMirrorPhase::RetiringForBackstop(previous),
                ActiveInjectionMirrorPhase::Installing => match self.previous.take() {
                    Some(previous) if !self.claimed_binding => {
                        ActiveInjectionMirrorPhase::Installed(previous)
                    }
                    Some(previous) => ActiveInjectionMirrorPhase::RetiringForBackstop(previous),
                    None => ActiveInjectionMirrorPhase::Open,
                },
                ActiveInjectionMirrorPhase::TerminalPendingInstall(_) => {
                    match self.previous.take() {
                        Some(previous) => ActiveInjectionMirrorPhase::RetiringForBackstop(previous),
                        None if self.claimed_binding => ActiveInjectionMirrorPhase::Closed,
                        None => ActiveInjectionMirrorPhase::Completed(
                            crate::channel_hooks::ImLiveMirrorAbortStatus::Unsafe,
                        ),
                    }
                }
                previous => previous,
            };
            // Any unfinished reservation can be the predecessor a registered
            // retry is waiting on, including an unclaimed same-binding path.
            true
        } else {
            false
        };
        if notify {
            self.coordinator.terminal_notify.notify_waiters();
        }
    }
}

/// Atomically reserve the late mirror slot for one exact active generation.
pub fn reserve_active_injection_im_mirror(
    session_id: &str,
    run_id: &str,
) -> LateInjectionMirrorReservation {
    let coordinator = INJECTION_CANCELS.lock().ok().and_then(|active| {
        active
            .get(session_id)
            .filter(|entry| entry.run_id == run_id)
            .map(|entry| entry.im_mirror.clone())
    });
    coordinator
        .map(|coordinator| coordinator.reserve_late())
        .unwrap_or(LateInjectionMirrorReservation::Stale)
}

async fn apply_injection_mirror_terminal(
    mirror: Box<dyn crate::channel_hooks::ImLiveMirror>,
    terminal: ActiveInjectionMirrorTerminal,
) -> crate::channel_hooks::ImLiveMirrorAbortStatus {
    match terminal {
        ActiveInjectionMirrorTerminal::Finalize(response) => {
            mirror.finalize(response).await;
            crate::channel_hooks::ImLiveMirrorAbortStatus::Confirmed
        }
        ActiveInjectionMirrorTerminal::Abort(body) => mirror.abort(body).await,
    }
}

enum CoordinatorTerminal {
    Complete(
        Box<dyn crate::channel_hooks::ImLiveMirror>,
        ActiveInjectionMirrorTerminal,
    ),
    RetireForBackstop(Box<dyn crate::channel_hooks::ImLiveMirror>),
    Wait,
    Done(crate::channel_hooks::ImLiveMirrorAbortStatus),
    NoMirror,
}

async fn terminalize_coordinated_injection_mirror(
    coordinator: &Arc<ActiveInjectionMirrorCoordinator>,
    terminal: ActiveInjectionMirrorTerminal,
) -> Option<crate::channel_hooks::ImLiveMirrorAbortStatus> {
    loop {
        let notified = coordinator.terminal_notify.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        let action = {
            let Ok(mut phase) = coordinator.phase.lock() else {
                return Some(crate::channel_hooks::ImLiveMirrorAbortStatus::Unsafe);
            };
            match std::mem::replace(&mut *phase, ActiveInjectionMirrorPhase::Closed) {
                ActiveInjectionMirrorPhase::Installed(mirror)
                    if coordinator.late_retry_waiters.load(Ordering::Acquire) > 0 =>
                {
                    *phase = ActiveInjectionMirrorPhase::Installed(mirror);
                    CoordinatorTerminal::Wait
                }
                ActiveInjectionMirrorPhase::Installed(mirror) => {
                    *phase = ActiveInjectionMirrorPhase::Completing;
                    CoordinatorTerminal::Complete(mirror, terminal.clone())
                }
                ActiveInjectionMirrorPhase::LateReplacing(previous) => {
                    *phase = ActiveInjectionMirrorPhase::TerminalPendingReplacement {
                        terminal: terminal.clone(),
                        previous,
                    };
                    CoordinatorTerminal::Wait
                }
                ActiveInjectionMirrorPhase::TerminalPendingReplacement {
                    terminal: existing,
                    previous,
                } => {
                    *phase = ActiveInjectionMirrorPhase::TerminalPendingReplacement {
                        terminal: existing,
                        previous,
                    };
                    CoordinatorTerminal::Wait
                }
                ActiveInjectionMirrorPhase::Installing => {
                    *phase = ActiveInjectionMirrorPhase::TerminalPendingInstall(terminal.clone());
                    CoordinatorTerminal::Wait
                }
                ActiveInjectionMirrorPhase::TerminalPendingInstall(existing) => {
                    *phase = ActiveInjectionMirrorPhase::TerminalPendingInstall(existing);
                    CoordinatorTerminal::Wait
                }
                ActiveInjectionMirrorPhase::Completing => {
                    *phase = ActiveInjectionMirrorPhase::Completing;
                    CoordinatorTerminal::Wait
                }
                ActiveInjectionMirrorPhase::RetiringForBackstop(mirror) => {
                    *phase = ActiveInjectionMirrorPhase::Completing;
                    CoordinatorTerminal::RetireForBackstop(mirror)
                }
                ActiveInjectionMirrorPhase::Completed(status) => {
                    *phase = ActiveInjectionMirrorPhase::Completed(status);
                    CoordinatorTerminal::Done(status)
                }
                ActiveInjectionMirrorPhase::Open
                    if coordinator.late_retry_waiters.load(Ordering::Acquire) > 0 =>
                {
                    *phase = ActiveInjectionMirrorPhase::Open;
                    CoordinatorTerminal::Wait
                }
                ActiveInjectionMirrorPhase::Initializing
                | ActiveInjectionMirrorPhase::LateRequested
                | ActiveInjectionMirrorPhase::Open
                | ActiveInjectionMirrorPhase::Closed => {
                    *phase = ActiveInjectionMirrorPhase::Closed;
                    CoordinatorTerminal::NoMirror
                }
            }
        };
        match action {
            CoordinatorTerminal::Complete(mirror, terminal) => {
                let status = apply_injection_mirror_terminal(mirror, terminal).await;
                let mut retry = false;
                if let Ok(mut phase) = coordinator.phase.lock() {
                    if coordinator.late_retry_waiters.load(Ordering::Acquire) > 0 {
                        *phase = ActiveInjectionMirrorPhase::Open;
                        retry = true;
                    } else {
                        *phase = ActiveInjectionMirrorPhase::Completed(status);
                    }
                }
                coordinator.terminal_notify.notify_waiters();
                if retry {
                    continue;
                }
                return Some(status);
            }
            CoordinatorTerminal::RetireForBackstop(mirror) => {
                let _ = mirror.abort(None).await;
                let mut retry = false;
                if let Ok(mut phase) = coordinator.phase.lock() {
                    if coordinator.late_retry_waiters.load(Ordering::Acquire) > 0 {
                        *phase = ActiveInjectionMirrorPhase::Open;
                        retry = true;
                    } else {
                        *phase = ActiveInjectionMirrorPhase::Closed;
                    }
                }
                coordinator.terminal_notify.notify_waiters();
                if retry {
                    continue;
                }
                return None;
            }
            CoordinatorTerminal::Wait => notified.as_mut().await,
            CoordinatorTerminal::Done(status) => return Some(status),
            CoordinatorTerminal::NoMirror => return None,
        }
    }
}

/// Complete the shared mirror when present; otherwise perform one Primary-side
/// terminal reattach. The backstop covers a binding written by another process
/// (or a lost attach notification) after this injection's initial `Absent`.
/// It runs before the durable source is settled and arms no-replay before the
/// first final provider mutation.
async fn terminalize_injection_im_delivery(
    session_id: &str,
    run_id: &str,
    coordinator: &Arc<ActiveInjectionMirrorCoordinator>,
    terminal: ActiveInjectionMirrorTerminal,
) -> crate::channel_hooks::ImLiveMirrorAbortStatus {
    if let Some(status) =
        terminalize_coordinated_injection_mirror(coordinator, terminal.clone()).await
    {
        return status;
    }

    let attach = crate::channel_hooks::attach_injection_mirror(session_id, run_id).await;
    match attach {
        crate::channel_hooks::ImLiveMirrorAttach::Attached(mirror) => {
            if !arm_injection_mirror_receipt(coordinator, run_id).await {
                let _ = mirror.abort(None).await;
                return crate::channel_hooks::ImLiveMirrorAbortStatus::Unsafe;
            }
            apply_injection_mirror_terminal(mirror, terminal).await
        }
        crate::channel_hooks::ImLiveMirrorAttach::Busy => {
            // A coordinator-managed installer would have put the phase in
            // `Installing`, making the first branch wait for it. Reaching Busy
            // here means an uncoordinated/stale provider claim exists; never
            // report it safe for automatic replay.
            app_warn!(
                "subagent",
                "inject::terminal_im_backstop",
                "Terminal IM attach for session {} run {} found an uncoordinated busy generation",
                session_id,
                run_id
            );
            crate::channel_hooks::ImLiveMirrorAbortStatus::Unsafe
        }
        crate::channel_hooks::ImLiveMirrorAttach::Absent
        | crate::channel_hooks::ImLiveMirrorAttach::Unavailable { .. }
        | crate::channel_hooks::ImLiveMirrorAttach::DeferredToPrimary { .. } => {
            crate::channel_hooks::ImLiveMirrorAbortStatus::Confirmed
        }
    }
}

fn settle_injection_source(receipt: Option<&OnInjected>, run_id: &str) {
    let Some(receipt) = receipt else { return };
    if let Err(error) = receipt.settle() {
        app_error!(
            "subagent",
            "inject",
            "Failed to persist terminal source receipt for run {}: {}",
            run_id,
            crate::logging::redact_sensitive(&error.to_string())
        );
    }
}

pub(crate) fn release_unarmed_injection_source(receipt: Option<&OnInjected>, run_id: &str) {
    let Some(receipt) = receipt else { return };
    if let Err(error) = receipt.release_unarmed() {
        app_error!(
            "subagent",
            "inject",
            "Failed to release unarmed source claim for run {}: {}",
            run_id,
            crate::logging::redact_sensitive(&error.to_string())
        );
    }
}

fn release_retry_source_if_abandoned(
    outcome: InjectionOutcome,
    receipt: Option<&OnInjected>,
    run_id: &str,
) {
    if matches!(outcome, InjectionOutcome::Abandoned) {
        release_unarmed_injection_source(receipt, run_id);
    }
}

fn can_defer_to_primary(receipt: Option<&OnInjected>) -> bool {
    receipt.is_some_and(OnInjected::supports_durable_handoff)
}

/// Establish the durable replay owner, persist the parent injection row, then
/// prepare the engine call in that strict order.
///
/// `persist` must contain every durable parent-session mutation that identifies
/// this attempt. A cross-process CAS loser (or any arm error) never invokes it,
/// and `start_engine` is invoked only after both arm and persistence succeed.
/// The receipt deliberately remains armed when persistence fails: callers must
/// treat that as a safe terminal failure instead of reviving startup replay.
fn arm_source_persist_then<T>(
    mirror_attached: bool,
    receipt: Option<&OnInjected>,
    persist: impl FnOnce() -> anyhow::Result<()>,
    start_engine: impl FnOnce(bool) -> T,
) -> anyhow::Result<T> {
    let mut no_replay_armed = receipt.is_some_and(OnInjected::is_no_replay_armed);
    if mirror_attached || receipt.is_some_and(OnInjected::requires_pre_engine_claim) {
        if let Some(receipt) = receipt {
            receipt.arm_no_replay()?;
            no_replay_armed = true;
        }
    }
    persist()?;
    Ok(start_engine(no_replay_armed))
}

/// Preserve retry idempotency without treating a failed dedup lookup as
/// "missing". In particular, a read error must not fall through to append a
/// second parent row.
fn persist_parent_injection_row_if_missing(
    already_written: impl FnOnce() -> anyhow::Result<bool>,
    append: impl FnOnce() -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    if already_written()? {
        Ok(())
    } else {
        append()
    }
}

/// Result of one `inject_and_run_parent` attempt. Lets the caller decide
/// whether the source record is done (`Injected`), owned by the retry queue
/// (`Queued`), or must stay pending for restart replay (`Abandoned`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjectionOutcome {
    /// Parent turn ran (or the result was already fetched / all models failed
    /// terminally), or a pre-engine persistence failure safely terminated an
    /// already-armed source. The source is settled, or its durable no-replay
    /// fence remains armed — nothing more should be replayed.
    Injected,
    /// Deferred: another injection holds the session, the user pre-empted this
    /// turn, or its IM delivery surface is not ready. The task lives in the
    /// unified per-session FIFO; the matching flush owns the retry. Caller must
    /// NOT mark the source injected.
    Queued,
    /// Could not safely hand the attempt to this process (for example a
    /// pre-engine durable arm failure or delegation to the Primary). Unless
    /// another process already owns the source, its replay marker remains
    /// pending for restart recovery (MISC-15: an abandoned injection must not
    /// look delivered). An unarmed parent-row read/write failure follows this
    /// path as well.
    Abandoned,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InjectionImTerminal {
    Failed,
    InterruptedWillRetry,
    InterruptedConsumed,
}

impl InjectionImTerminal {
    /// Static, user-facing IM copy only. Raw provider / model errors must never
    /// cross this boundary because they may contain credentials or request
    /// details. Detailed diagnostics remain in the local session event below.
    fn body(self) -> &'static str {
        match self {
            Self::Failed => {
                "⚠️ **Background follow-up failed** — this reply has stopped. Please try again later."
            }
            Self::InterruptedWillRetry => {
                "⏸️ **Background follow-up interrupted** — a new message took priority. It will retry automatically when the conversation is idle."
            }
            Self::InterruptedConsumed => {
                "⏹️ **Background follow-up stopped** — the result was already retrieved in this conversation, so it will not retry."
            }
        }
    }
}

struct ParentInjectionSink {
    parent_session_id: String,
    run_id: String,
}

impl crate::chat_engine::EventSink for ParentInjectionSink {
    fn send(&self, event: &str) {
        emit_parent_stream_event(&ParentAgentStreamEvent {
            event_type: "delta".into(),
            parent_session_id: self.parent_session_id.clone(),
            run_id: self.run_id.clone(),
            push_message: None,
            delta: Some(event.to_string()),
            error: None,
        });
    }
}

/// A deferred injection task that was cancelled and needs to be retried.
#[derive(Clone)]
pub(super) struct PendingInjection {
    pub parent_session_id: String,
    pub parent_agent_id: String,
    pub child_agent_id: String,
    pub run_id: String,
    pub push_message: String,
    pub session_db: Arc<crate::session::SessionDB>,
    /// Carried so a deferred injection still marks its source done when the
    /// queued attempt eventually lands. `None` means this result has no durable
    /// receipt and must never be abandoned for cross-process replay.
    pub on_injected: Option<OnInjected>,
    /// Keeps a verified first-party HTTP UI approval path alive while this
    /// parent follow-up waits behind a foreground turn.
    pub reattachable_ui_guard: Option<crate::permission::ReattachableUiSessionGuard>,
    /// Readiness gate at this task's exact FIFO position. A blocked head must
    /// not be skipped in favour of a later task for the same session.
    gate: PendingInjectionGate,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum PendingInjectionGate {
    Ready,
    Channel { account_id: Option<String> },
}

fn same_pending_identity(left: &PendingInjection, right: &PendingInjection) -> bool {
    left.parent_session_id == right.parent_session_id && left.run_id == right.run_id
}

/// A newly discovered B always joins the tail; if A is currently active or
/// already pending for that session, admission must not let B bypass it.
fn enqueue_new_pending_tail(queue: &mut Vec<PendingInjection>, task: PendingInjection) {
    if !queue
        .iter()
        .any(|queued| same_pending_identity(queued, &task))
    {
        queue.push(task);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InjectionAdmission {
    Claimed,
    Queued,
    Coalesced,
}

/// Decide one fresh dispatch under the caller's shared INJECTING -> PENDING
/// critical section. An exact active/pending identity is already owned and is
/// coalesced; a distinct run for the same session joins the FIFO tail.
fn admit_or_enqueue_injection(
    injecting: &mut std::collections::HashMap<String, String>,
    queue: &mut Vec<PendingInjection>,
    task: &PendingInjection,
) -> InjectionAdmission {
    let session_id = task.parent_session_id.clone();
    let run_id = task.run_id.clone();
    if injecting
        .get(&session_id)
        .is_some_and(|active_run| active_run == &run_id)
        || queue
            .iter()
            .any(|queued| same_pending_identity(queued, &task))
    {
        return InjectionAdmission::Coalesced;
    }
    if injecting.contains_key(&session_id)
        || queue
            .iter()
            .any(|queued| queued.parent_session_id == session_id)
    {
        enqueue_new_pending_tail(queue, task.clone());
        return InjectionAdmission::Queued;
    }
    injecting.insert(session_id, run_id);
    InjectionAdmission::Claimed
}

/// Active A retries before every B that arrived while A owned the session.
/// Remove a defensive duplicate first, then insert at the first same-session
/// position (or the global tail when A has no suffix).
fn enqueue_active_retry_front(queue: &mut Vec<PendingInjection>, task: PendingInjection) {
    queue.retain(|queued| !same_pending_identity(queued, &task));
    let index = queue
        .iter()
        .position(|queued| queued.parent_session_id == task.parent_session_id)
        .unwrap_or(queue.len());
    queue.insert(index, task);
}

/// Queue an active attempt without opening an admission gap. Every caller uses
/// the global INJECTING -> PENDING lock order, so a concurrent new B either
/// observes A active and appends, or observes A already at the FIFO head.
fn requeue_active_injection(task: PendingInjection) {
    let _injecting = INJECTING_SESSIONS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut queue = PENDING_INJECTIONS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    enqueue_active_retry_front(&mut queue, task);
}

fn open_channel_gates(
    queue: &mut [PendingInjection],
    ready_account_id: Option<&str>,
) -> Vec<String> {
    let mut sessions = std::collections::HashSet::new();
    for task in queue {
        let PendingInjectionGate::Channel { account_id } = &task.gate else {
            continue;
        };
        let matches_ready = ready_account_id
            .is_none_or(|ready| account_id.as_deref().is_none_or(|account| account == ready));
        if matches_ready {
            task.gate = PendingInjectionGate::Ready;
            sessions.insert(task.parent_session_id.clone());
        }
    }
    sessions.into_iter().collect()
}

/// Open matching Channel gates after a delivery-surface event. `Some(id)`
/// selects that account plus unknown bindings; `None` is reserved for a lagged
/// full recheck. Tasks stay in their original FIFO positions.
pub(crate) fn flush_channel_pending_injections(ready_account_id: Option<&str>) {
    let sessions = {
        let mut pending = PENDING_INJECTIONS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        open_channel_gates(&mut pending, ready_account_id)
    };
    for session_id in sessions {
        flush_pending_injections(&session_id);
    }
}

fn persisted_surface_releases_channel_gate(
    persisted_enabled: Option<bool>,
    account_is_running: bool,
) -> bool {
    match persisted_enabled {
        None | Some(false) => true,
        Some(true) => account_is_running,
    }
}

/// Close start/disable/remove event-before-enqueue after a task acquires its
/// Channel gate. Unknown account ids stay blocked: repeatedly opening them
/// without a resolvable surface would turn an infrastructure failure into a
/// tight retry loop.
async fn channel_gate_should_recheck_now(account_id: Option<&str>) -> bool {
    let Some(account_id) = account_id else {
        return false;
    };
    let persisted_enabled = crate::config::cached_config()
        .channels
        .find_account(account_id)
        .map(|account| account.enabled);
    let account_is_running = if persisted_enabled == Some(true) {
        match crate::globals::get_channel_registry() {
            Some(registry) => registry.health(account_id).await.is_running,
            None => false,
        }
    } else {
        false
    };
    persisted_surface_releases_channel_gate(persisted_enabled, account_is_running)
}

fn claim_next_pending_injection(
    queue: &mut Vec<PendingInjection>,
    injecting: &mut std::collections::HashMap<String, String>,
    session_id: &str,
) -> Option<PendingInjection> {
    if injecting.contains_key(session_id) {
        return None;
    }
    let index = queue
        .iter()
        .position(|task| task.parent_session_id == session_id)?;
    if queue[index].gate != PendingInjectionGate::Ready {
        return None;
    }
    let task = queue.remove(index);
    injecting.insert(session_id.to_string(), task.run_id.clone());
    Some(task)
}

/// Drop every deferred ParentInjection for a deleted/purged session. This
/// includes both ordinary idle retries and Channel-gated work because they now
/// share one ownership/FIFO structure.
pub(crate) fn purge_pending_for_session(session_id: &str) -> usize {
    let mut queue = PENDING_INJECTIONS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    purge_pending_from_queue(&mut queue, session_id)
}

fn purge_pending_from_queue(queue: &mut Vec<PendingInjection>, session_id: &str) -> usize {
    let before = queue.len();
    let mut process_claims = Vec::new();
    queue.retain(|task| {
        if task.parent_session_id == session_id {
            if let Some(receipt) = task.on_injected.as_ref() {
                process_claims.push(receipt.clone());
            }
            false
        } else {
            true
        }
    });
    // This callback is constrained to process-local, non-blocking cleanup. Do
    // not call `release_unarmed` here: subagent receipts may write SQLite, and
    // this function is reached from the async session-cleanup watcher.
    for receipt in process_claims {
        receipt.release_process_dispatch();
    }
    before.saturating_sub(queue.len())
}

/// Claim and re-trigger the next pending injection for a session.
/// Called from ChatSessionGuard::drop when a user chat completes.
pub(crate) fn flush_pending_injections(session_id: &str) {
    loop {
        // Atomically (under the established INJECTING -> PENDING lock order)
        // remove one matching task and reserve the session for it. Without the
        // preclaim, two concurrent CleanupGuard / ChatSessionGuard drops could
        // dequeue A and B before either spawned runtime registered itself,
        // rotating the same-session FIFO suffix when B re-queued.
        let task = {
            let mut injecting = INJECTING_SESSIONS.lock().unwrap_or_else(|p| p.into_inner());
            let mut queue = match PENDING_INJECTIONS.lock() {
                Ok(q) => q,
                Err(p) => p.into_inner(),
            };
            claim_next_pending_injection(&mut queue, &mut injecting, session_id)
        };
        let Some(task) = task else { return };

        // Skip if already fetched, and clean up the entry
        let already_fetched = {
            let mut set = FETCHED_RUN_IDS.lock().unwrap_or_else(|p| p.into_inner());
            set.remove(&task.run_id)
        };
        if already_fetched {
            let released = {
                let mut injecting = INJECTING_SESSIONS.lock().unwrap_or_else(|p| p.into_inner());
                let _queue = PENDING_INJECTIONS.lock().unwrap_or_else(|p| p.into_inner());
                release_injection_owner(&mut injecting, session_id, &task.run_id)
            };
            if !released {
                return;
            }
            continue;
        }
        let t = task.clone();
        let retry_receipt = t.on_injected.clone();
        let retry_run_id = t.run_id.clone();
        let preclaimed_cleanup = CleanupGuard {
            session_id: t.parent_session_id.clone(),
            run_id: t.run_id.clone(),
        };
        std::thread::spawn(move || {
            // Own the dequeue-time session reservation until the async
            // injection returns. If thread/runtime construction fails, dropping
            // the captured guard still releases the claim and advances FIFO.
            let mut preclaimed_cleanup = Some(preclaimed_cleanup);
            match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => {
                    let outcome = rt.block_on(inject_and_run_parent_with_ui_guard(
                        t.parent_session_id,
                        t.parent_agent_id,
                        t.child_agent_id,
                        t.run_id,
                        t.push_message,
                        t.session_db,
                        t.on_injected,
                        t.reattachable_ui_guard,
                        preclaimed_cleanup.take(),
                    ));
                    // A pre-provider failure consumes this retry task without
                    // firing its receipt. Release only an ordinary unarmed
                    // source claim; the callback and shared armed bit both
                    // refuse `injecting_no_replay`.
                    release_retry_source_if_abandoned(
                        outcome,
                        retry_receipt.as_ref(),
                        &retry_run_id,
                    );
                }
                Err(e) => {
                    app_error!(
                        "subagent",
                        "inject",
                        "Failed to build runtime for retry: {}",
                        e
                    );
                    release_unarmed_injection_source(retry_receipt.as_ref(), &retry_run_id);
                }
            }
        });
        return; // Next task stays queued until this one's CleanupGuard fires.
    }
}

/// Build the push message text injected into the parent session.
pub(crate) fn build_subagent_push_message(
    thread_id: &str,
    run_id: &str,
    agent_id: &str,
    task: &str,
    status: &SubagentStatus,
    duration_ms: u64,
    result: Option<&str>,
    error: Option<&str>,
    terminal_reason: Option<crate::subagent::SubagentTerminalReason>,
) -> String {
    let duration = format!("{:.1}s", duration_ms as f64 / 1000.0);
    let result_block = result
        .filter(|text| !text.trim().is_empty())
        .map(|text| format!("<result>\n{}\n</result>\n", escape_xml_text(text.trim())))
        .unwrap_or_default();
    let error_block = error
        .filter(|text| !text.trim().is_empty())
        .map(|text| format!("<error>\n{}\n</error>\n", escape_xml_text(text.trim())))
        .unwrap_or_default();
    let output_block = if result_block.is_empty() && error_block.is_empty() {
        "<result>(no output)</result>\n".to_string()
    } else {
        format!("{}{}", result_block, error_block)
    };
    let summary = format!(
        "Sub-agent \"{}\" finished with status \"{}\" in {}.",
        agent_id,
        status.as_str(),
        duration
    );
    let terminal_reason =
        terminal_reason.unwrap_or(crate::subagent::SubagentTerminalReason::Unknown);
    format!(
        "<subagent-result>\n\
         <thread-id>{}</thread-id>\n\
         <run-id>{}</run-id>\n\
         <agent>{}</agent>\n\
         <status>{}</status>\n\
         <terminal-reason>{}</terminal-reason>\n\
         <resume-allowed>{}</resume-allowed>\n\
         <resume-recommended>{}</resume-recommended>\n\
         <duration-ms>{}</duration-ms>\n\
         <duration>{}</duration>\n\
         <task>{}</task>\n\
         {}\
         <summary>{}</summary>\n\
         </subagent-result>",
        escape_xml_text(thread_id),
        escape_xml_text(run_id),
        escape_xml_text(agent_id),
        escape_xml_text(status.as_str()),
        escape_xml_text(terminal_reason.as_str()),
        terminal_reason.resume_allowed(),
        terminal_reason.resume_recommended(),
        duration_ms,
        escape_xml_text(&duration),
        escape_xml_text(&truncate_str(task, 50)),
        output_block,
        escape_xml_text(&summary)
    )
}

fn escape_xml_text(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// E2 / DELETE-3 / INCOG-3 backstop: is the parent session still present?
/// An absent row (deleted or incognito-burned) must abort the injection before
/// it resurrects a ghost turn (a billed LLM round + persisted rows against a
/// session that no longer exists). A transient lookup error is treated as
/// *alive* so a momentary glitch doesn't drop a real injection —
/// `dispatch_injection` already fired the primary gate, and the idle-timeout
/// path leaves the source row for restart replay.
fn parent_session_present(db: &crate::session::SessionDB, session_id: &str) -> bool {
    !matches!(db.get_session(session_id), Ok(None))
}

fn session_autonomy_fence(
    db: &crate::session::SessionDB,
    session_id: &str,
) -> anyhow::Result<(u64, bool)> {
    Ok((
        db.session_autonomy_lineage_pause_epoch(session_id)?,
        db.is_session_or_ancestor_autonomy_paused(session_id)?,
    ))
}

/// `child_agent_id` label used by `crate::wakeup` when reusing this injection
/// pipeline for a self-scheduled wakeup (R10). `inject_and_run_parent` branches
/// on it to write a `wakeup_trigger` marker instead of `subagent_result`.
pub(crate) const WAKEUP_CHILD_AGENT_ID: &str = "wakeup";
pub(crate) const PROCESS_NOTIFICATION_CHILD_AGENT_ID: &str = "process_notification";
pub const LOOP_CHILD_AGENT_ID: &str = "loop";
pub(crate) const WORKFLOW_CHILD_AGENT_ID: &str = "workflow";

/// Outcome of waiting for a parent session to become idle before injecting.
enum IdleWait {
    /// No foreground turn is active — safe to inject now.
    Idle,
    /// `should_abort` fired (e.g. the agent already fetched the result via a
    /// `check`/`result` tool action) — caller treats the injection as handled.
    Aborted,
    /// Timed out waiting for the session to go idle — caller abandons the
    /// attempt (the source row stays for restart replay).
    TimedOut,
}

/// Wait until `session_id` has no active foreground chat turn, or until
/// `should_abort` fires, or `max_wait` elapses.
///
/// Foreground turns are tracked in `ACTIVE_CHAT_SESSIONS` by
/// [`ChatSessionGuard`](super::ChatSessionGuard), created at the shared
/// `run_chat_engine` entry (R2) so this gate holds across desktop / HTTP / IM /
/// cron — and at the ACP turn boundary for ACP. The wait is event-driven on
/// `SESSION_IDLE_NOTIFY` (fired when a guard releases) with a bounded fallback
/// poll so a missed notification can't park forever. The fallback is clamped to
/// the time remaining before `max_wait` so the timeout is honored promptly
/// regardless of the 5s poll cadence.
async fn wait_for_session_idle(
    session_id: &str,
    max_wait: std::time::Duration,
    should_abort: impl Fn() -> bool,
) -> IdleWait {
    let fallback_interval = std::time::Duration::from_secs(5);
    let start = std::time::Instant::now();
    loop {
        let is_busy = ACTIVE_CHAT_SESSIONS
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(session_id)
            .copied()
            .unwrap_or(0)
            > 0;
        if !is_busy {
            return IdleWait::Idle;
        }
        if start.elapsed() >= max_wait {
            return IdleWait::TimedOut;
        }
        if should_abort() {
            return IdleWait::Aborted;
        }
        // Wait for notify (instant wake) or the fallback poll (in case notify is
        // missed). Cap the poll at the remaining budget so timeout is honored
        // without overshooting by up to a full poll interval.
        let remaining = max_wait.saturating_sub(start.elapsed());
        let sleep_dur = fallback_interval.min(remaining.max(std::time::Duration::from_millis(1)));
        tokio::select! {
            _ = SESSION_IDLE_NOTIFY.notified() => {}
            _ = tokio::time::sleep(sleep_dur) => {}
        }
    }
}

/// Backend-driven result injection: wait for idle, then run the parent agent with the push message.
/// Respects user chat priority: waits if busy, cancels if user sends a new message, skips if
/// the agent already fetched the result via check/result tool actions.
pub async fn inject_and_run_parent(
    parent_session_id: String,
    parent_agent_id: String,
    child_agent_id: String,
    run_id: String,
    push_message: String,
    session_db: Arc<crate::session::SessionDB>,
    on_injected: Option<OnInjected>,
) -> InjectionOutcome {
    inject_and_run_parent_with_ui_guard(
        parent_session_id,
        parent_agent_id,
        child_agent_id,
        run_id,
        push_message,
        session_db,
        on_injected,
        None,
        None,
    )
    .await
}

/// Variant used by first-party UI descendant work. The lease is moved into
/// `PENDING_INJECTIONS` whenever delivery is deferred, so closing/reloading the
/// browser never converts a later parent follow-up approval into unattended.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn inject_and_run_parent_with_ui_guard(
    parent_session_id: String,
    parent_agent_id: String,
    child_agent_id: String,
    run_id: String,
    push_message: String,
    session_db: Arc<crate::session::SessionDB>,
    on_injected: Option<OnInjected>,
    reattachable_ui_guard: Option<crate::permission::ReattachableUiSessionGuard>,
    preclaimed_cleanup: Option<CleanupGuard>,
) -> InjectionOutcome {
    use crate::provider;

    let session_preclaimed = preclaimed_cleanup.is_some();
    let mut _cleanup = preclaimed_cleanup;

    // 0. Skip if the parent agent already fetched this result via check/result tool
    {
        let mut set = FETCHED_RUN_IDS.lock().unwrap_or_else(|p| p.into_inner());
        if set.contains(&run_id) {
            app_info!(
                "subagent",
                "inject",
                "Run {} already fetched by parent, skipping injection",
                &run_id
            );
            set.remove(&run_id); // Clean up — no longer needed
            settle_injection_source(on_injected.as_ref(), &run_id);
            return InjectionOutcome::Injected;
        }
    }

    // E2 / DELETE-3 / INCOG-3 backstop (entry): mirror dispatch_injection's gate
    // in case the session was already gone by the time this attempt starts. Fire
    // `on_injected` (consume the source so replay won't retry a dead session)
    // and bail — this is `Injected`, not `Abandoned`.
    if !parent_session_present(&session_db, &parent_session_id) {
        app_info!(
            "subagent",
            "inject",
            "Parent session {} gone; skipping injection for run {}",
            &parent_session_id,
            &run_id
        );
        settle_injection_source(on_injected.as_ref(), &run_id);
        return InjectionOutcome::Injected;
    }

    // Capture a monotonic Stop generation before the potentially long idle
    // wait. An active flag alone is insufficient: Stop followed quickly by
    // Continue could clear it while this pre-Stop injector is still parked.
    // The generation never decreases, so such an injector must abandon and let
    // the durable source be reclaimed after Continue.
    let admitted_pause_epoch = match session_autonomy_fence(&session_db, &parent_session_id) {
        Ok((epoch, false)) => epoch,
        Ok((_, true)) => {
            app_info!(
                "subagent",
                "inject",
                "Deferring parent injection {} while session {} is paused",
                run_id,
                parent_session_id
            );
            release_unarmed_injection_source(on_injected.as_ref(), &run_id);
            return InjectionOutcome::Abandoned;
        }
        Err(error) => {
            app_warn!(
                "subagent",
                "inject",
                "Cannot read Stop fence for parent injection {} in session {}: {}",
                run_id,
                parent_session_id,
                error
            );
            release_unarmed_injection_source(on_injected.as_ref(), &run_id);
            return InjectionOutcome::Abandoned;
        }
    };

    // Admit against both active ownership and the existing FIFO under one lock
    // critical section. A new B must never claim the session while an older A
    // is queued (including a Channel-gated A).
    if !session_preclaimed {
        let mut injecting = INJECTING_SESSIONS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut queue = PENDING_INJECTIONS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let admission = admit_or_enqueue_injection(
            &mut injecting,
            &mut queue,
            &PendingInjection {
                parent_session_id: parent_session_id.clone(),
                parent_agent_id: parent_agent_id.clone(),
                child_agent_id: child_agent_id.clone(),
                run_id: run_id.clone(),
                push_message: push_message.clone(),
                session_db: session_db.clone(),
                on_injected: on_injected.clone(),
                reattachable_ui_guard: reattachable_ui_guard.clone(),
                gate: PendingInjectionGate::Ready,
            },
        );
        match admission {
            InjectionAdmission::Coalesced => {
                app_debug!(
                    "subagent",
                    "inject",
                    "Coalesced duplicate parent injection for session {} run {}",
                    &parent_session_id,
                    &run_id
                );
                return InjectionOutcome::Queued;
            }
            InjectionAdmission::Queued => {
                app_info!(
                    "subagent",
                    "inject",
                    "Session {} already has a different active/pending injection; queued run {}",
                    &parent_session_id,
                    &run_id
                );
                return InjectionOutcome::Queued;
            }
            InjectionAdmission::Claimed => {}
        }
        drop(queue);
        drop(injecting);
        _cleanup = Some(CleanupGuard {
            session_id: parent_session_id.clone(),
            run_id: run_id.clone(),
        });
    }

    // 1. Wait for parent session to become idle (event-driven with timeout
    // fallback). The idle gate (`ACTIVE_CHAT_SESSIONS`) is now populated by
    // `ChatSessionGuard` at the shared `run_chat_engine` entry (R2), so this
    // wait correctly parks behind live turns on every entry point, not just
    // desktop.
    let announce_timeout = crate::agent_loader::load_agent(&parent_agent_id)
        .ok()
        .and_then(|def| def.config.subagents.announce_timeout_secs)
        .unwrap_or(120)
        .clamp(10, 600);
    let max_wait = std::time::Duration::from_secs(announce_timeout);
    match wait_for_session_idle(&parent_session_id, max_wait, || {
        // Re-check if the result was fetched while we were waiting.
        FETCHED_RUN_IDS
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .contains(&run_id)
    })
    .await
    {
        IdleWait::Idle => {}
        IdleWait::TimedOut => {
            // G3/G5: the parent session stayed busy past `announce_timeout`.
            // Re-queue (carrying `on_injected`) instead of abandoning to
            // restart-replay — `PENDING_INJECTIONS` flushes when the long
            // foreground turn ends (`ChatSessionGuard::drop`), so the completion
            // surfaces this run instead of waiting for the next process start.
            // Critical for subagent / Group injections (`on_injected = None`),
            // which have no `injected=0` restart-replay backstop — a Group's
            // merged injection (row `injected=true`, out of replay) would
            // otherwise be lost permanently. `on_injected` is carried but NOT
            // fired, so a tool job's row stays un-injected (MISC-15: an
            // undelivered injection must not look delivered) and the restart
            // backstop is preserved.
            app_warn!(
                "subagent",
                "inject",
                "Session {} still busy after idle wait; re-queuing injection for run {}",
                &parent_session_id,
                &run_id
            );
            requeue_active_injection(PendingInjection {
                parent_session_id,
                parent_agent_id,
                child_agent_id,
                run_id,
                push_message,
                session_db,
                on_injected,
                reattachable_ui_guard,
                gate: PendingInjectionGate::Ready,
            });
            return InjectionOutcome::Queued;
        }
        IdleWait::Aborted => {
            app_info!(
                "subagent",
                "inject",
                "Run {} fetched while waiting, skipping",
                &run_id
            );
            settle_injection_source(on_injected.as_ref(), &run_id);
            return InjectionOutcome::Injected;
        }
    }

    // Final check before proceeding
    if FETCHED_RUN_IDS
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .contains(&run_id)
    {
        settle_injection_source(on_injected.as_ref(), &run_id);
        return InjectionOutcome::Injected;
    }

    // 2. Register cancel flag — user's chat() will set this to abort the injection
    let cancel = Arc::new(AtomicBool::new(false));
    let im_mirror_coordinator =
        Arc::new(ActiveInjectionMirrorCoordinator::new(on_injected.clone()));
    let mut initial_im_mirror_resolution =
        InitialInjectionMirrorResolutionGuard::new(im_mirror_coordinator.clone());
    if let Ok(mut map) = INJECTION_CANCELS.lock() {
        map.insert(
            parent_session_id.clone(),
            ActiveInjection {
                run_id: run_id.clone(),
                cancel: cancel.clone(),
                admitted_pause_epoch,
                im_mirror: im_mirror_coordinator.clone(),
            },
        );
    }
    // Ensure cancel flag is cleaned up on all exit paths
    let cancel_cleanup_sid = parent_session_id.clone();
    struct CancelCleanup {
        sid: String,
    }
    impl Drop for CancelCleanup {
        fn drop(&mut self) {
            if let Ok(mut map) = INJECTION_CANCELS.lock() {
                map.remove(&self.sid);
            }
        }
    }
    let _cancel_cleanup = CancelCleanup {
        sid: cancel_cleanup_sid,
    };

    // 3. Emit "started" so frontend can show loading state
    emit_parent_stream_event(&ParentAgentStreamEvent {
        event_type: "started".into(),
        parent_session_id: parent_session_id.clone(),
        run_id: run_id.clone(),
        push_message: Some(push_message.clone()),
        delta: None,
        error: None,
    });

    // 4. Build model chain
    let store = crate::config::cached_config();
    let agent_model_config = crate::agent_loader::load_agent(&parent_agent_id)
        .map(|def| def.config.model)
        .unwrap_or_default();
    let (primary, fallbacks) = provider::resolve_model_chain(&agent_model_config, &store);
    let mut model_chain = Vec::new();
    if let Some(p) = primary {
        model_chain.push(p);
    }
    for fb in fallbacks {
        if !model_chain.iter().any(|m: &crate::provider::ActiveModel| {
            m.provider_id == fb.provider_id && m.model_id == fb.model_id
        }) {
            model_chain.push(fb);
        }
    }

    if model_chain.is_empty() {
        app_error!(
            "subagent",
            "inject",
            "No model configured for parent agent {}",
            &parent_agent_id
        );
        emit_parent_stream_event(&ParentAgentStreamEvent {
            event_type: "error".into(),
            parent_session_id: parent_session_id.clone(),
            run_id: run_id.clone(),
            push_message: None,
            delta: None,
            error: Some("No model configured for parent agent".into()),
        });
        // Persistent misconfiguration: mark injected so a restart doesn't
        // re-inject in a loop. The tool output is still saved to disk; only
        // the notification is dropped, and the parent can't run without a model.
        settle_injection_source(on_injected.as_ref(), &run_id);
        return InjectionOutcome::Injected;
    }

    let mut last_error = String::new();
    let mut succeeded = false;
    // Captured at the engine-error boundary so the IM terminal copy and the
    // later queue decision use the same fetched/not-fetched observation.
    let mut cancelled_while_running_fetched: Option<bool> = None;
    let mut engine_failed_without_cancel = false;
    let mut im_terminal_safe_for_retry = true;
    // E2 / DELETE-3 / INCOG-3 backstop (post-idle): the most dangerous window —
    // the session can be deleted or burned *during* the idle wait above. Re-check
    // before writing the push row or running a billed turn against a dead session.
    if !parent_session_present(&session_db, &parent_session_id) {
        app_info!(
            "subagent",
            "inject",
            "Parent session {} gone during idle wait; skipping injection for run {}",
            &parent_session_id,
            &run_id
        );
        settle_injection_source(on_injected.as_ref(), &run_id);
        return InjectionOutcome::Injected;
    }

    match session_autonomy_fence(&session_db, &parent_session_id) {
        Ok((epoch, false)) if epoch == admitted_pause_epoch => {}
        Ok((epoch, paused)) => {
            app_info!(
                "subagent",
                "inject",
                "Abandoning stale parent injection {} for session {} after Stop fence changed (admitted_epoch={} current_epoch={} paused={})",
                run_id,
                parent_session_id,
                admitted_pause_epoch,
                epoch,
                paused
            );
            release_unarmed_injection_source(on_injected.as_ref(), &run_id);
            return InjectionOutcome::Abandoned;
        }
        Err(error) => {
            app_warn!(
                "subagent",
                "inject",
                "Cannot revalidate Stop fence for parent injection {} in session {}: {}",
                run_id,
                parent_session_id,
                error
            );
            release_unarmed_injection_source(on_injected.as_ref(), &run_id);
            return InjectionOutcome::Abandoned;
        }
    }

    // Acquire after the potentially long idle wait but before writing the push
    // row. This closes the terminal-subagent/delete race without pinning the
    // Agent for the entire wait; the engine keeps its own admission backstop.
    let _agent_admission = match crate::agent_lifecycle::begin_agent_run(&parent_agent_id) {
        Ok(guard) => guard,
        Err(error) => {
            app_warn!(
                "subagent",
                "inject",
                "Parent agent {} became unavailable before injection {}: {}",
                &parent_agent_id,
                &run_id,
                error
            );
            return InjectionOutcome::Abandoned;
        }
    };

    // The foreground HTTP turn may already have returned. Keep the dormant
    // eval root identity alive while this real parent-injection turn runs so
    // its model/tool calls remain in the originating trial rather than
    // becoming unattributed background usage.
    let _eval_injection_guard = crate::eval_context::retain_session(&parent_session_id);

    if cancel.load(Ordering::SeqCst) {
        app_info!(
            "subagent",
            "inject",
            "Injection cancelled before attempt for session {}",
            &parent_session_id
        );
    } else {
        let parent_agent_def = crate::agent_loader::load_agent(&parent_agent_id).ok();

        // G1: if the parent session is attached to an IM chat, mirror this
        // injection turn into it so an IM-origin background task's completion
        // reaches the IM user (per the account's `imReplyMode`). Reuses the
        // GUI↔IM live mirror; the engine's own attach gates `ParentInjection`
        // out, so we drive it here and AWAIT finalize/abort below — this runs on
        // a short-lived current-thread runtime whose drop would cancel a spawned
        // finalize. `None` when there's no IM attach (desktop-only / no channel).
        let injection_attach =
            crate::channel_hooks::attach_injection_mirror(&parent_session_id, &run_id).await;
        let injection_has_delivery_surface = match injection_attach {
            crate::channel_hooks::ImLiveMirrorAttach::Absent => {
                let _ = initial_im_mirror_resolution.resolve(None);
                false
            }
            crate::channel_hooks::ImLiveMirrorAttach::Attached(mirror) => {
                if !initial_im_mirror_resolution.resolve(Some(mirror)) {
                    app_error!(
                        "subagent",
                        "inject",
                        "Initial IM mirror coordinator rejected attached generation for session {} run {}",
                        &parent_session_id,
                        &run_id
                    );
                    return InjectionOutcome::Abandoned;
                }
                true
            }
            crate::channel_hooks::ImLiveMirrorAttach::Unavailable { account_id } => {
                initial_im_mirror_resolution.close();
                // A durable IM binding exists, but its account worker is still
                // starting. Do not append the parent row, arm no-replay, or
                // settle its receipt. Every source shape (including in-memory
                // process/group notifications) waits in the same per-session
                // FIFO behind a Channel gate; durable sources retain their
                // original crash recovery.
                app_info!(
                    "subagent",
                    "inject",
                    "IM mirror unavailable for parent session {}; queuing run {} for account readiness",
                    &parent_session_id,
                    &run_id
                );
                let health_account_id = account_id.clone();
                requeue_active_injection(PendingInjection {
                    parent_session_id,
                    parent_agent_id,
                    child_agent_id,
                    run_id,
                    push_message,
                    session_db,
                    on_injected,
                    reattachable_ui_guard,
                    gate: PendingInjectionGate::Channel { account_id },
                });

                // Close event-before-enqueue: account startup may have emitted
                // its event just before this task acquired the FIFO. Re-check
                // both readiness directions: running opens Attached; a
                // persisted remove/disable opens GUI-only Absent.
                if channel_gate_should_recheck_now(health_account_id.as_deref()).await {
                    flush_channel_pending_injections(health_account_id.as_deref());
                }
                return InjectionOutcome::Queued;
            }
            crate::channel_hooks::ImLiveMirrorAttach::DeferredToPrimary { account_id } => {
                if !can_defer_to_primary(on_injected.as_ref()) {
                    // Group merges, hook feedback, process notifications, and
                    // similar process-local sources have no durable row the
                    // Primary sweep can rediscover. Preserve their only copy by
                    // running the parent turn locally without an IM mutation.
                    app_warn!(
                        "subagent",
                        "inject",
                        "Parent injection run {} has no durable Primary handoff; continuing GUI-only in Secondary (account={})",
                        &run_id,
                        account_id.as_deref().unwrap_or("unknown")
                    );
                    let _ = initial_im_mirror_resolution.resolve(None);
                    false
                } else {
                    initial_im_mirror_resolution.close();
                    app_info!(
                        "subagent",
                        "inject",
                        "Deferring parent injection run {} to Primary IM owner (account={})",
                        &run_id,
                        account_id.as_deref().unwrap_or("unknown")
                    );
                    emit_parent_stream_event(&ParentAgentStreamEvent {
                        event_type: "error".into(),
                        parent_session_id,
                        run_id,
                        push_message: None,
                        delta: None,
                        error: Some(
                            "Background follow-up was deferred because durable IM delivery is owned by the Primary process"
                                .into(),
                        ),
                    });
                    return InjectionOutcome::Abandoned;
                }
            }
            crate::channel_hooks::ImLiveMirrorAttach::Busy => {
                // A binding change may have claimed this exact generation while
                // the initial channel hook was resolving. The shared coordinator
                // owns its terminal handoff, so this is a delivery surface — not
                // a duplicate parent-model attempt to abandon.
                app_info!(
                    "subagent",
                    "inject",
                    "IM mirror generation already owned for parent session {} run {}; continuing with external terminal owner",
                    &parent_session_id,
                    &run_id
                );
                let _ = initial_im_mirror_resolution.resolve(None);
                true
            }
        };

        // Attach first to determine whether this attempt has an external IM
        // mutation surface. If so, claim its durable replay source before
        // writing *any* parent user row; a cross-process CAS loser must leave
        // the session untouched. With no mirror, this deliberately skips the
        // arm and retains the existing at-least-once restart contract.
        let resolved_reasoning_effort = parent_agent_def
            .as_ref()
            .and_then(|def| def.config.model.reasoning_effort.clone())
            .or(crate::agent::live_reasoning_effort(None).await);
        let engine_params = crate::chat_engine::ChatEngineParams {
            session_id: parent_session_id.clone(),
            agent_id: parent_agent_id.clone(),
            turn_id: None,
            message: push_message.clone(),
            incoming_turn: None,
            display_text: None,
            attachments: Vec::new(),
            session_db: session_db.clone(),
            model_chain,
            providers: store.providers.clone(),
            codex_token: None,
            resolved_temperature: parent_agent_def
                .as_ref()
                .and_then(|def| def.config.model.temperature)
                .or(store.temperature),
            compact_config: store.compact.clone(),
            run_context: None,
            reasoning_effort: resolved_reasoning_effort,
            cancel: cancel.clone(),
            plan_context_override: None,
            skill_allowed_tools: Vec::new(),
            denied_tools: Vec::new(),
            tool_scope: None,
            subagent_depth: 0,
            steer_run_id: None,
            auto_approve_tools: false,
            follow_global_reasoning_effort: false,
            post_turn_effects: false,
            abort_on_cancel: true,
            persist_final_error_event: false,
            source: crate::chat_engine::stream_seq::ChatSource::ParentInjection,
            ui_surface: None,
            origin_source: None,
            // Parent-injection turns are owner-internal, never IM. No opt-in gate.
            channel_kb_context: None,
            event_sink: Arc::new(ParentInjectionSink {
                parent_session_id: parent_session_id.clone(),
                run_id: run_id.clone(),
            }),
        };
        let engine_result = arm_source_persist_then(
            injection_has_delivery_surface,
            on_injected.as_ref(),
            || {
                // Write the push user row BEFORE agent.chat() so intermediate
                // rows streamed from the callback land between it and the final
                // assistant row. Re-queued attempts retain the run_id and reuse
                // this idempotency guard.
                persist_parent_injection_row_if_missing(
                    || session_db.has_injection_user_msg(&parent_session_id, &run_id),
                    || {
                        let mut user_msg = crate::session::NewMessage::user(&push_message)
                            .with_source(crate::chat_engine::ChatSource::ParentInjection);
                        // A wakeup is a trigger rather than a subagent result.
                        // Every shape retains run_id because the dedup lookup
                        // above uses it to recognize confirmed in-process retries.
                        let meta = if child_agent_id == WAKEUP_CHILD_AGENT_ID {
                            serde_json::json!({ "wakeup_trigger": { "run_id": &run_id } })
                        } else if child_agent_id == LOOP_CHILD_AGENT_ID {
                            serde_json::json!({ "loop_trigger": { "run_id": &run_id } })
                        } else if child_agent_id == PROCESS_NOTIFICATION_CHILD_AGENT_ID {
                            serde_json::json!({ "process_notification": { "run_id": &run_id } })
                        } else if child_agent_id == WORKFLOW_CHILD_AGENT_ID {
                            serde_json::json!({ "workflow_result": { "run_id": &run_id } })
                        } else {
                            serde_json::json!({
                                "subagent_result": {
                                    "run_id": &run_id,
                                    "agent_id": &child_agent_id,
                                }
                            })
                        };
                        user_msg.attachments_meta = Some(meta.to_string());
                        session_db
                            .append_injection_user_msg_if_missing(
                                &parent_session_id,
                                &run_id,
                                &user_msg,
                            )
                            .map(|_| ())
                    },
                )
            },
            |_armed| crate::chat_engine::run_chat_engine(engine_params),
        );
        let engine = match engine_result {
            Ok(engine) => engine,
            Err(error) => {
                let durable_no_replay_armed = on_injected
                    .as_ref()
                    .is_some_and(OnInjected::is_no_replay_armed);
                app_error!(
                    "subagent",
                    "inject",
                    "Failed to prepare parent injection for run {} (no_replay_armed={}): {}",
                    &run_id,
                    durable_no_replay_armed,
                    crate::logging::redact_sensitive(&error.to_string())
                );
                let _ = terminalize_coordinated_injection_mirror(
                    &im_mirror_coordinator,
                    ActiveInjectionMirrorTerminal::Abort(None),
                )
                .await;
                emit_parent_stream_event(&ParentAgentStreamEvent {
                    event_type: "error".into(),
                    parent_session_id,
                    run_id,
                    push_message: None,
                    delta: None,
                    error: Some(
                        "Background follow-up was not started because its delivery state could not be saved"
                            .into(),
                    ),
                });
                // Never settle here. If arm succeeded, its durable no-replay
                // fence is the terminal safety decision; reviving the source
                // could duplicate a provider-side reply after a crash. Without
                // such a fence (including the no-mirror path), leave the source
                // pending for the existing at-least-once restart replay.
                return if durable_no_replay_armed {
                    InjectionOutcome::Injected
                } else {
                    InjectionOutcome::Abandoned
                };
            }
        };

        match engine.await {
            Ok(result) => {
                // run_chat_engine returning Ok means the reply was persisted.
                // Mark succeeded unconditionally — even if cancel flipped to
                // true after Ok was produced (user started new chat in the
                // narrow post-return window), re-queueing would write a
                // duplicate sub-agent completion to the parent conversation.
                let model_label = result
                    .model_used
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_else(|| "(unknown model)".to_string());
                app_info!(
                    "subagent",
                    "inject",
                    "Parent agent {} responded via model {}",
                    &parent_agent_id,
                    model_label
                );
                succeeded = true;
                crate::eval_context::record_lifecycle_event(
                    Some(&parent_session_id),
                    "handoff",
                    "agent.result_injected",
                    Some(&run_id),
                    "completed",
                    0,
                );
                // G1: deliver the mirrored injection turn to IM (per imReplyMode).
                // Awaited so it completes before this current-thread runtime drops.
                let _ = terminalize_injection_im_delivery(
                    &parent_session_id,
                    &run_id,
                    &im_mirror_coordinator,
                    ActiveInjectionMirrorTerminal::Finalize(result.response.clone()),
                )
                .await;
                // G2: if this is a cron run session, fan the injected result out to
                // the cron job's delivery_targets (the inline run delivered its own
                // response; a background job spawned during the run completes later
                // and would otherwise reach nobody). No-op for non-cron sessions.
                crate::cron_hooks::deliver_injection_for_session(
                    &parent_session_id,
                    &result.response,
                )
                .await;
            }
            Err(e) => {
                let was_cancelled = cancel.load(Ordering::SeqCst);
                let fetched_while_active = was_cancelled
                    && FETCHED_RUN_IDS
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .contains(&run_id);
                let terminal = if was_cancelled {
                    cancelled_while_running_fetched = Some(fetched_while_active);
                    app_info!(
                        "subagent",
                        "inject",
                        "Injection cancelled (error path) for session {} (result_fetched={})",
                        &parent_session_id,
                        fetched_while_active
                    );
                    if fetched_while_active {
                        InjectionImTerminal::InterruptedConsumed
                    } else {
                        InjectionImTerminal::InterruptedWillRetry
                    }
                } else {
                    engine_failed_without_cancel = true;
                    last_error = e;
                    InjectionImTerminal::Failed
                };
                // G1: a ParentInjection has no user-quote, but its Message/Card
                // preview can already be visible. Terminate that same preview
                // identity with bounded static copy before a retry can create a
                // second reply. Native mirrors use their provider abort path.
                im_terminal_safe_for_retry = terminalize_injection_im_delivery(
                    &parent_session_id,
                    &run_id,
                    &im_mirror_coordinator,
                    ActiveInjectionMirrorTerminal::Abort(Some(terminal.body().to_string())),
                )
                .await
                .is_confirmed();
            }
        }
    }

    let retry_provider_failure = engine_failed_without_cancel
        && im_terminal_safe_for_retry
        && on_injected.as_ref().is_some_and(|receipt| {
            receipt.supports_unarmed_retry() && !receipt.is_no_replay_armed()
        });

    // A no-replay/ephemeral source cannot be retried safely. Persist its error;
    // ordinary durable sources return to the replay pool below instead.
    if engine_failed_without_cancel && !retry_provider_failure {
        let _ = session_db.append_message(
            &parent_session_id,
            &crate::session::NewMessage::error_event(&format!("[injection failed] {}", last_error))
                .with_source(crate::chat_engine::ChatSource::ParentInjection),
        );
    }

    // 6. Emit final event. Order matters: a successful Ok already persisted
    // the reply, so even if cancel was set after the run completed, we must
    // not re-queue (would duplicate the sub-agent completion in the parent
    // conversation).
    let was_cancelled =
        !succeeded && !engine_failed_without_cancel && cancel.load(Ordering::SeqCst);
    let fetched_while_active = cancelled_while_running_fetched.unwrap_or_else(|| {
        FETCHED_RUN_IDS
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .contains(&run_id)
    });
    if was_cancelled && !im_terminal_safe_for_retry {
        // A persistent provider mutation may still be visible. Starting a new
        // mirror for this result would risk partial + full double delivery, so
        // keep the write-ahead source fence armed. The durable child/job result
        // remains inspectable, but neither the current process nor startup
        // recovery may send it again automatically.
        app_warn!(
            "subagent",
            "inject",
            "Injection for run {} was cancelled but its IM mirror terminal is unconfirmed; automatic retry suppressed",
            &run_id
        );
        crate::eval_context::record_lifecycle_event(
            Some(&parent_session_id),
            "handoff",
            "agent.result_injected",
            Some(&run_id),
            "terminal_ambiguous_no_replay",
            0,
        );
        emit_parent_stream_event(&ParentAgentStreamEvent {
            event_type: "error".into(),
            parent_session_id,
            run_id,
            push_message: None,
            delta: None,
            error: Some(
                "Cancelled: previous IM reply could not be closed safely; automatic retry was suppressed"
                    .into(),
            ),
        });
        InjectionOutcome::Injected
    } else if was_cancelled && fetched_while_active {
        settle_injection_source(on_injected.as_ref(), &run_id);
        emit_parent_stream_event(&ParentAgentStreamEvent {
            event_type: "done".into(),
            parent_session_id,
            run_id,
            push_message: None,
            delta: None,
            error: None,
        });
        InjectionOutcome::Injected
    } else if was_cancelled {
        // Re-queue for retry after the user's chat completes, carrying
        // on_injected so the eventual landing still marks the source done.
        requeue_active_injection(PendingInjection {
            parent_session_id: parent_session_id.clone(),
            parent_agent_id: parent_agent_id.clone(),
            child_agent_id,
            run_id: run_id.clone(),
            push_message,
            session_db,
            on_injected,
            reattachable_ui_guard,
            gate: PendingInjectionGate::Ready,
        });
        app_info!(
            "subagent",
            "inject",
            "Injection for run {} cancelled, re-queued for next idle",
            &run_id
        );
        emit_parent_stream_event(&ParentAgentStreamEvent {
            event_type: "error".into(),
            parent_session_id,
            run_id,
            push_message: None,
            delta: None,
            error: Some("Cancelled: user started new chat, will retry when idle".into()),
        });
        InjectionOutcome::Queued
    } else if succeeded {
        settle_injection_source(on_injected.as_ref(), &run_id);
        emit_parent_stream_event(&ParentAgentStreamEvent {
            event_type: "done".into(),
            parent_session_id,
            run_id,
            push_message: None,
            delta: None,
            error: None,
        });
        InjectionOutcome::Injected
    } else if retry_provider_failure {
        release_unarmed_injection_source(on_injected.as_ref(), &run_id);
        let retry_config = crate::agent_loader::load_agent(&parent_agent_id)
            .map(|definition| definition.config.subagents)
            .unwrap_or_default();
        let max_retries = retry_config.provider_retry_attempts.min(10);
        let base_delay_secs = retry_config.provider_retry_backoff_secs.clamp(1, 60);
        // `app_init::spawn_delivery_surface_replay_listener` runs the durable
        // due-delivery sweep every five seconds (and after restart recovery),
        // so persisting `requested_at` is sufficient to schedule this retry.
        // Keep this DB-owned instead of adding a competing per-run timer.
        let deferred = match session_db.defer_subagent_result_delivery_retry(
            &run_id,
            &last_error,
            max_retries,
            base_delay_secs,
        ) {
            Ok(deferred) => deferred,
            Err(error) => {
                app_warn!(
                    "subagent",
                    "inject",
                    "Failed to defer parent delivery retry for run {}: {}",
                    run_id,
                    error
                );
                false
            }
        };
        emit_parent_stream_event(&ParentAgentStreamEvent {
            event_type: "error".into(),
            parent_session_id,
            run_id,
            push_message: None,
            delta: None,
            error: Some(if deferred {
                "All parent models failed; durable result delivery will retry".into()
            } else {
                "All parent models failed; automatic result delivery retries are exhausted".into()
            }),
        });
        InjectionOutcome::Abandoned
    } else {
        // A provider mutation may have crossed the no-replay boundary, or this
        // source has no durable replay receipt. Settle rather than risk a
        // duplicate external reply.
        settle_injection_source(on_injected.as_ref(), &run_id);
        emit_parent_stream_event(&ParentAgentStreamEvent {
            event_type: "error".into(),
            parent_session_id,
            run_id,
            push_message: None,
            delta: None,
            error: Some(format!("All models failed: {}", last_error)),
        });
        InjectionOutcome::Injected
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[derive(Default)]
    struct MirrorProbe {
        finalized: Arc<AtomicUsize>,
        aborted: Arc<AtomicUsize>,
    }

    struct ProbeMirror {
        probe: MirrorProbe,
    }

    impl crate::channel_hooks::ImLiveMirror for ProbeMirror {
        fn finalize(
            self: Box<Self>,
            _response: String,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>> {
            Box::pin(async move {
                self.probe.finalized.fetch_add(1, Ordering::SeqCst);
            })
        }

        fn abort(
            self: Box<Self>,
            _body: Option<String>,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = crate::channel_hooks::ImLiveMirrorAbortStatus>
                    + Send
                    + 'static,
            >,
        > {
            Box::pin(async move {
                self.probe.aborted.fetch_add(1, Ordering::SeqCst);
                crate::channel_hooks::ImLiveMirrorAbortStatus::Confirmed
            })
        }
    }

    fn probe_mirror(probe: &MirrorProbe) -> Box<dyn crate::channel_hooks::ImLiveMirror> {
        Box::new(ProbeMirror {
            probe: MirrorProbe {
                finalized: probe.finalized.clone(),
                aborted: probe.aborted.clone(),
            },
        })
    }

    #[test]
    fn no_replay_arm_is_single_flight_across_receipt_clones() {
        let calls = Arc::new(AtomicUsize::new(0));
        let receipt = OnInjected::new(
            {
                let calls = calls.clone();
                move || {
                    calls.fetch_add(1, Ordering::SeqCst);
                    std::thread::sleep(std::time::Duration::from_millis(20));
                    Ok(())
                }
            },
            || Ok(()),
        );
        std::thread::scope(|scope| {
            for _ in 0..8 {
                let receipt = receipt.clone();
                scope.spawn(move || receipt.arm_no_replay().expect("arm receipt"));
            }
        });
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn late_request_waits_for_initial_absent_without_losing_wakeup() {
        let coordinator = Arc::new(ActiveInjectionMirrorCoordinator::new(None));
        let LateInjectionMirrorReservation::Reserved(mut install) = coordinator.reserve_late()
        else {
            panic!("late reservation");
        };
        let arm = tokio::spawn(async move {
            let armed = install.arm_no_replay("run-a").await;
            (armed, install)
        });
        tokio::task::yield_now().await;
        assert!(coordinator.resolve_initial(None));
        let (armed, install) = tokio::time::timeout(std::time::Duration::from_secs(1), arm)
            .await
            .expect("arm wakeup")
            .expect("arm task");
        assert!(armed);

        let probe = MirrorProbe::default();
        assert!(install.install(probe_mirror(&probe)).await);
        let status = terminalize_coordinated_injection_mirror(
            &coordinator,
            ActiveInjectionMirrorTerminal::Finalize("done".into()),
        )
        .await;
        assert!(status.is_some_and(|status| status.is_confirmed()));
        assert_eq!(probe.finalized.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn closing_initial_attach_wakes_waiting_late_reservation() {
        let coordinator = Arc::new(ActiveInjectionMirrorCoordinator::new(None));
        let LateInjectionMirrorReservation::Reserved(mut install) = coordinator.reserve_late()
        else {
            panic!("late reservation");
        };
        let arm = tokio::spawn(async move { install.arm_no_replay("run-closed").await });
        tokio::task::yield_now().await;

        coordinator.close_initial();

        let armed = tokio::time::timeout(std::time::Duration::from_secs(1), arm)
            .await
            .expect("close wakeup")
            .expect("arm task");
        assert!(!armed);
    }

    #[tokio::test]
    async fn unresolved_initial_guard_wakes_late_reservation_on_early_exit() {
        let coordinator = Arc::new(ActiveInjectionMirrorCoordinator::new(None));
        let initial = InitialInjectionMirrorResolutionGuard::new(coordinator.clone());
        let LateInjectionMirrorReservation::Reserved(mut install) = coordinator.reserve_late()
        else {
            panic!("late reservation");
        };
        let arm = tokio::spawn(async move { install.arm_no_replay("run-early-exit").await });
        tokio::task::yield_now().await;

        // Models any return between registration in `INJECTION_CANCELS` and
        // completion of the initial channel attach.
        drop(initial);

        let armed = tokio::time::timeout(std::time::Duration::from_secs(1), arm)
            .await
            .expect("guard wakeup")
            .expect("arm task");
        assert!(!armed);
    }

    #[tokio::test]
    async fn resolved_initial_guard_preserves_attached_terminal_owner() {
        let coordinator = Arc::new(ActiveInjectionMirrorCoordinator::new(None));
        let probe = MirrorProbe::default();
        {
            let mut initial = InitialInjectionMirrorResolutionGuard::new(coordinator.clone());
            assert!(initial.resolve(Some(probe_mirror(&probe))));
        }

        let status = terminalize_coordinated_injection_mirror(
            &coordinator,
            ActiveInjectionMirrorTerminal::Finalize("done".into()),
        )
        .await;
        assert!(status.is_some_and(|status| status.is_confirmed()));
        assert_eq!(probe.finalized.load(Ordering::SeqCst), 1);
        assert_eq!(probe.aborted.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn terminal_waits_for_reserved_late_install_and_finalizes_once() {
        let coordinator = Arc::new(ActiveInjectionMirrorCoordinator::new(None));
        assert!(coordinator.resolve_initial(None));
        let LateInjectionMirrorReservation::Reserved(mut install) = coordinator.reserve_late()
        else {
            panic!("late reservation");
        };
        assert!(install.arm_no_replay("run-terminal-race").await);

        let terminal_coordinator = coordinator.clone();
        let terminal = tokio::spawn(async move {
            terminalize_coordinated_injection_mirror(
                &terminal_coordinator,
                ActiveInjectionMirrorTerminal::Finalize("done".into()),
            )
            .await
        });
        tokio::task::yield_now().await;
        let probe = MirrorProbe::default();
        assert!(install.install(probe_mirror(&probe)).await);
        let status = tokio::time::timeout(std::time::Duration::from_secs(1), terminal)
            .await
            .expect("terminal wakeup")
            .expect("terminal task");
        assert!(status.is_some_and(|status| status.is_confirmed()));
        assert_eq!(probe.finalized.load(Ordering::SeqCst), 1);
        assert_eq!(probe.aborted.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn rebind_retires_old_owner_before_new_owner_gets_terminal() {
        let coordinator = Arc::new(ActiveInjectionMirrorCoordinator::new(None));
        let old = MirrorProbe::default();
        let new = MirrorProbe::default();
        assert!(coordinator.resolve_initial(Some(probe_mirror(&old))));
        let LateInjectionMirrorReservation::Reserved(mut install) = coordinator.reserve_late()
        else {
            panic!("replacement reservation");
        };
        assert!(install.arm_no_replay("run-rebind").await);
        assert!(install.retire_previous().await);
        assert_eq!(old.aborted.load(Ordering::SeqCst), 1);
        assert!(install.install(probe_mirror(&new)).await);

        let status = terminalize_coordinated_injection_mirror(
            &coordinator,
            ActiveInjectionMirrorTerminal::Finalize("new target".into()),
        )
        .await;
        assert!(status.is_some_and(|status| status.is_confirmed()));
        assert_eq!(old.finalized.load(Ordering::SeqCst), 0);
        assert_eq!(new.finalized.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn overlapping_rebind_busy_retries_before_terminal_consumes_intermediate_owner() {
        let coordinator = Arc::new(ActiveInjectionMirrorCoordinator::new(None));
        let first = MirrorProbe::default();
        let intermediate = MirrorProbe::default();
        let latest = MirrorProbe::default();
        assert!(coordinator.resolve_initial(Some(probe_mirror(&first))));

        let LateInjectionMirrorReservation::Reserved(mut install_intermediate) =
            coordinator.reserve_late()
        else {
            panic!("intermediate reservation");
        };
        assert!(install_intermediate.retire_previous().await);
        assert_eq!(first.aborted.load(Ordering::SeqCst), 1);

        let LateInjectionMirrorReservation::Busy(latest_retry) = coordinator.reserve_late() else {
            panic!("latest rebind must register a retry while B installs");
        };
        assert!(
            install_intermediate
                .install(probe_mirror(&intermediate))
                .await
        );

        let terminal_coordinator = coordinator.clone();
        let terminal = tokio::spawn(async move {
            terminalize_coordinated_injection_mirror(
                &terminal_coordinator,
                ActiveInjectionMirrorTerminal::Finalize("latest target".into()),
            )
            .await
        });
        tokio::task::yield_now().await;
        assert_eq!(intermediate.finalized.load(Ordering::SeqCst), 0);

        let LateInjectionMirrorReservation::Reserved(mut install_latest) =
            tokio::time::timeout(std::time::Duration::from_secs(1), latest_retry.wait())
                .await
                .expect("latest retry wakeup")
        else {
            panic!("latest retry reservation");
        };
        assert!(install_latest.retire_previous().await);
        assert_eq!(intermediate.aborted.load(Ordering::SeqCst), 1);
        assert!(install_latest.install(probe_mirror(&latest)).await);

        let status = tokio::time::timeout(std::time::Duration::from_secs(1), terminal)
            .await
            .expect("terminal wakeup")
            .expect("terminal task");
        assert!(status.is_some_and(|status| status.is_confirmed()));
        assert_eq!(first.finalized.load(Ordering::SeqCst), 0);
        assert_eq!(intermediate.finalized.load(Ordering::SeqCst), 0);
        assert_eq!(latest.finalized.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn late_claim_takes_over_initial_attach_that_resolves_stale() {
        let coordinator = Arc::new(ActiveInjectionMirrorCoordinator::new(None));
        let old = MirrorProbe::default();
        let new = MirrorProbe::default();
        let LateInjectionMirrorReservation::Reserved(mut install) = coordinator.reserve_late()
        else {
            panic!("replacement reservation");
        };

        // The late path has already claimed the newer attach generation when
        // the slower initial hook returns an older target.
        assert!(coordinator.resolve_initial(Some(probe_mirror(&old))));
        let terminal_coordinator = coordinator.clone();
        let terminal = tokio::spawn(async move {
            terminalize_coordinated_injection_mirror(
                &terminal_coordinator,
                ActiveInjectionMirrorTerminal::Finalize("new target".into()),
            )
            .await
        });
        loop {
            let pending = coordinator.phase.lock().is_ok_and(|phase| {
                matches!(
                    *phase,
                    ActiveInjectionMirrorPhase::TerminalPendingReplacement { .. }
                )
            });
            if pending {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(old.finalized.load(Ordering::SeqCst), 0);

        assert!(install.retire_previous().await);
        assert_eq!(old.aborted.load(Ordering::SeqCst), 1);
        assert!(install.arm_no_replay("run-stale-initial").await);
        assert!(install.install(probe_mirror(&new)).await);

        let status = tokio::time::timeout(std::time::Duration::from_secs(1), terminal)
            .await
            .expect("terminal wakeup")
            .expect("terminal task");
        assert!(status.is_some_and(|status| status.is_confirmed()));
        assert_eq!(old.finalized.load(Ordering::SeqCst), 0);
        assert_eq!(new.finalized.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn unclaimed_late_reservation_keeps_initial_terminal_owner() {
        let coordinator = Arc::new(ActiveInjectionMirrorCoordinator::new(None));
        let old = MirrorProbe::default();
        let LateInjectionMirrorReservation::Reserved(install) = coordinator.reserve_late() else {
            panic!("late reservation");
        };
        assert!(coordinator.resolve_initial(Some(probe_mirror(&old))));

        let terminal_coordinator = coordinator.clone();
        let terminal = tokio::spawn(async move {
            terminalize_coordinated_injection_mirror(
                &terminal_coordinator,
                ActiveInjectionMirrorTerminal::Finalize("initial target".into()),
            )
            .await
        });
        loop {
            let pending = coordinator.phase.lock().is_ok_and(|phase| {
                matches!(
                    *phase,
                    ActiveInjectionMirrorPhase::TerminalPendingReplacement { .. }
                )
            });
            if pending {
                break;
            }
            tokio::task::yield_now().await;
        }

        // Models the newer path losing `try_claim` with Busy.
        drop(install);
        let status = tokio::time::timeout(std::time::Duration::from_secs(1), terminal)
            .await
            .expect("terminal wakeup")
            .expect("terminal task");
        assert!(status.is_some_and(|status| status.is_confirmed()));
        assert_eq!(old.finalized.load(Ordering::SeqCst), 1);
        assert_eq!(old.aborted.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn failed_rebind_arm_does_not_restore_retired_owner() {
        let receipt = OnInjected::new(|| Err(anyhow::anyhow!("temporary arm failure")), || Ok(()));
        let coordinator = Arc::new(ActiveInjectionMirrorCoordinator::new(Some(receipt)));
        let old = MirrorProbe::default();
        assert!(coordinator.resolve_initial(Some(probe_mirror(&old))));
        let LateInjectionMirrorReservation::Reserved(mut install) = coordinator.reserve_late()
        else {
            panic!("replacement reservation");
        };

        assert!(install.retire_previous().await);
        assert!(!install.arm_no_replay("run-arm-failed").await);
        drop(install);

        let status = terminalize_coordinated_injection_mirror(
            &coordinator,
            ActiveInjectionMirrorTerminal::Finalize("current target".into()),
        )
        .await;
        assert!(status.is_none());
        assert_eq!(old.finalized.load(Ordering::SeqCst), 0);
        assert_eq!(old.aborted.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn dropped_rebind_installer_retires_old_and_requests_backstop() {
        let coordinator = Arc::new(ActiveInjectionMirrorCoordinator::new(None));
        let old = MirrorProbe::default();
        assert!(coordinator.resolve_initial(Some(probe_mirror(&old))));
        let LateInjectionMirrorReservation::Reserved(install) = coordinator.reserve_late() else {
            panic!("replacement reservation");
        };

        let terminal_coordinator = coordinator.clone();
        let terminal = tokio::spawn(async move {
            terminalize_coordinated_injection_mirror(
                &terminal_coordinator,
                ActiveInjectionMirrorTerminal::Finalize("old target".into()),
            )
            .await
        });
        loop {
            let pending = coordinator.phase.lock().is_ok_and(|phase| {
                matches!(
                    *phase,
                    ActiveInjectionMirrorPhase::TerminalPendingInstall(_)
                )
            });
            if pending {
                break;
            }
            tokio::task::yield_now().await;
        }
        drop(install);

        let status = tokio::time::timeout(std::time::Duration::from_secs(1), terminal)
            .await
            .expect("terminal wakeup")
            .expect("terminal task");
        assert!(status.is_none());
        assert_eq!(old.finalized.load(Ordering::SeqCst), 0);
        assert_eq!(old.aborted.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn subagent_push_message_uses_xmlish_payload_and_escapes_text() {
        let msg = build_subagent_push_message(
            "thread<&",
            "run<&",
            "agent>&",
            "read <file> & report",
            &SubagentStatus::Completed,
            1234,
            Some("ok <done> & safe"),
            None,
            Some(crate::subagent::SubagentTerminalReason::Success),
        );

        assert!(msg.starts_with("<subagent-result>"));
        assert!(msg.contains("<thread-id>thread&lt;&amp;</thread-id>"));
        assert!(msg.contains("<run-id>run&lt;&amp;</run-id>"));
        assert!(msg.contains("<agent>agent&gt;&amp;</agent>"));
        assert!(msg.contains("<task>read &lt;file&gt; &amp; report</task>"));
        assert!(msg.contains("<result>\nok &lt;done&gt; &amp; safe\n</result>"));
        assert!(!msg.contains("BEGIN_SUBAGENT_RESULT"));
    }

    #[test]
    fn injection_im_terminal_copy_is_static_and_describes_retry_semantics() {
        let failed = InjectionImTerminal::Failed.body();
        let retry = InjectionImTerminal::InterruptedWillRetry.body();
        let consumed = InjectionImTerminal::InterruptedConsumed.body();

        assert!(failed.contains("failed"));
        assert!(retry.contains("retry automatically"));
        assert!(consumed.contains("will not retry"));

        // The IM copy is selected from a closed enum and never interpolates the
        // raw engine error, provider response, token, or request URL.
        for body in [failed, retry, consumed] {
            assert!(!body.contains("sk-test-secret"));
            assert!(!body.contains("provider.example"));
        }
    }

    #[test]
    fn durable_arm_failure_never_persists_parent_row_or_starts_engine() {
        let steps = Arc::new(std::sync::Mutex::new(Vec::new()));
        let arm_steps = steps.clone();
        let receipt = OnInjected::new(
            move || {
                arm_steps.lock().unwrap().push("arm");
                anyhow::bail!("durable CAS lost")
            },
            || Ok(()),
        );
        let persist_steps = steps.clone();
        let engine_steps = steps.clone();

        let result = arm_source_persist_then(
            true,
            Some(&receipt),
            move || {
                persist_steps.lock().unwrap().push("persist-parent-row");
                Ok(())
            },
            move |_| engine_steps.lock().unwrap().push("start-engine"),
        );

        assert!(result.is_err());
        assert_eq!(*steps.lock().unwrap(), ["arm"]);
        assert!(!receipt.is_no_replay_armed());
    }

    #[test]
    fn armed_parent_dedup_read_failure_skips_append_and_engine() {
        let receipt = OnInjected::new(|| Ok(()), || Ok(()));
        let append_called = Arc::new(AtomicBool::new(false));
        let append_flag = append_called.clone();
        let engine_started = Arc::new(AtomicBool::new(false));
        let engine_flag = engine_started.clone();

        let result = arm_source_persist_then(
            true,
            Some(&receipt),
            || {
                persist_parent_injection_row_if_missing(
                    || anyhow::bail!("dedup read failed"),
                    move || {
                        append_flag.store(true, Ordering::SeqCst);
                        Ok(())
                    },
                )
            },
            move |_| engine_flag.store(true, Ordering::SeqCst),
        );

        assert!(result.is_err());
        assert!(!append_called.load(Ordering::SeqCst));
        assert!(!engine_started.load(Ordering::SeqCst));
        assert!(receipt.is_no_replay_armed());
    }

    #[test]
    fn armed_parent_write_failure_keeps_fence_and_never_starts_engine() {
        let steps = Arc::new(std::sync::Mutex::new(Vec::new()));
        let arm_steps = steps.clone();
        let receipt = OnInjected::new(
            move || {
                arm_steps.lock().unwrap().push("arm");
                Ok(())
            },
            || Ok(()),
        );
        let persist_steps = steps.clone();
        let engine_steps = steps.clone();

        let result = arm_source_persist_then(
            true,
            Some(&receipt),
            move || {
                persist_parent_injection_row_if_missing(
                    || {
                        persist_steps.lock().unwrap().push("read-parent-row");
                        Ok(false)
                    },
                    || {
                        persist_steps.lock().unwrap().push("persist-parent-row");
                        anyhow::bail!("parent row write failed")
                    },
                )
            },
            move |_| engine_steps.lock().unwrap().push("start-engine"),
        );

        assert!(result.is_err());
        assert_eq!(
            *steps.lock().unwrap(),
            ["arm", "read-parent-row", "persist-parent-row"]
        );
        assert!(receipt.is_no_replay_armed());
    }

    #[test]
    fn no_mirror_keeps_source_replayable_until_parent_turn_settles() {
        let steps = Arc::new(std::sync::Mutex::new(Vec::new()));
        let arm_steps = steps.clone();
        let receipt = OnInjected::new(
            move || {
                arm_steps.lock().unwrap().push("arm");
                Ok(())
            },
            || Ok(()),
        );
        let persist_steps = steps.clone();
        let engine_steps = steps.clone();

        let armed = arm_source_persist_then(
            false,
            Some(&receipt),
            move || {
                persist_steps.lock().unwrap().push("persist-parent-row");
                Ok(())
            },
            move |armed| {
                engine_steps.lock().unwrap().push("start-engine");
                armed
            },
        )
        .unwrap();

        assert!(!armed);
        assert_eq!(
            *steps.lock().unwrap(),
            ["persist-parent-row", "start-engine"]
        );
        assert!(!receipt.is_no_replay_armed());
    }

    #[test]
    fn pre_engine_claim_fences_cross_process_source_without_mirror() {
        let steps = Arc::new(std::sync::Mutex::new(Vec::new()));
        let arm_steps = steps.clone();
        let receipt = OnInjected::new(
            move || {
                arm_steps.lock().unwrap().push("arm");
                Ok(())
            },
            || Ok(()),
        )
        .with_pre_engine_claim();
        let persist_steps = steps.clone();
        let engine_steps = steps.clone();

        let armed = arm_source_persist_then(
            false,
            Some(&receipt),
            move || {
                persist_steps.lock().unwrap().push("persist-parent-row");
                Ok(())
            },
            move |armed| {
                engine_steps.lock().unwrap().push("start-engine");
                armed
            },
        )
        .unwrap();

        assert!(armed);
        assert_eq!(
            *steps.lock().unwrap(),
            ["arm", "persist-parent-row", "start-engine"]
        );
        assert!(receipt.is_no_replay_armed());
    }

    #[test]
    fn queued_retry_abandon_releases_only_an_unarmed_claim() {
        let releases = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let release_counter = releases.clone();
        let receipt = OnInjected::new(|| Ok(()), || Ok(())).with_release_unarmed(move || {
            release_counter.fetch_add(1, Ordering::SeqCst);
            Ok(())
        });

        release_retry_source_if_abandoned(
            InjectionOutcome::Abandoned,
            Some(&receipt),
            "run-unarmed",
        );
        release_retry_source_if_abandoned(
            InjectionOutcome::Abandoned,
            Some(&receipt),
            "run-unarmed",
        );
        assert_eq!(
            releases.load(Ordering::SeqCst),
            1,
            "release callback must be idempotent"
        );

        let armed_releases = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let armed_release_counter = armed_releases.clone();
        let armed = OnInjected::new(|| Ok(()), || Ok(())).with_release_unarmed(move || {
            armed_release_counter.fetch_add(1, Ordering::SeqCst);
            Ok(())
        });
        armed.arm_no_replay().unwrap();
        release_retry_source_if_abandoned(InjectionOutcome::Abandoned, Some(&armed), "run-armed");
        assert_eq!(
            armed_releases.load(Ordering::SeqCst),
            0,
            "an armed no-replay source must never be released"
        );
    }

    #[test]
    fn process_dispatch_claim_follows_queued_injected_and_abandoned_outcomes() {
        let queued_releases = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let queued_counter = queued_releases.clone();
        let queued =
            OnInjected::new(|| Ok(()), || Ok(())).with_process_dispatch_release(move || {
                queued_counter.fetch_add(1, Ordering::SeqCst);
            });
        assert_eq!(
            queued_releases.load(Ordering::SeqCst),
            0,
            "moving a receipt into PendingInjection must retain its dispatch claim"
        );

        let settled_releases = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let settled_counter = settled_releases.clone();
        let settled =
            OnInjected::new(|| Ok(()), || Ok(())).with_process_dispatch_release(move || {
                settled_counter.fetch_add(1, Ordering::SeqCst);
            });
        settled.settle().unwrap();
        assert_eq!(settled_releases.load(Ordering::SeqCst), 1);

        let armed_releases = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let armed_counter = armed_releases.clone();
        let armed =
            OnInjected::new(|| Ok(()), || Ok(())).with_process_dispatch_release(move || {
                armed_counter.fetch_add(1, Ordering::SeqCst);
            });
        armed.arm_no_replay().unwrap();
        assert_eq!(armed_releases.load(Ordering::SeqCst), 1);

        let retained_releases = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let retained_counter = retained_releases.clone();
        let retained = OnInjected::new(|| Ok(()), || Ok(()))
            .with_process_dispatch_release(move || {
                retained_counter.fetch_add(1, Ordering::SeqCst);
            })
            .retain_process_dispatch_until_settle();
        retained.arm_no_replay().unwrap();
        assert_eq!(
            retained_releases.load(Ordering::SeqCst),
            0,
            "retained claim must stay owner-visible while the engine runs"
        );
        retained.settle().unwrap();
        assert_eq!(retained_releases.load(Ordering::SeqCst), 1);

        let abandoned_releases = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let abandoned_counter = abandoned_releases.clone();
        let abandoned =
            OnInjected::new(|| Ok(()), || Ok(())).with_process_dispatch_release(move || {
                abandoned_counter.fetch_add(1, Ordering::SeqCst);
            });
        release_retry_source_if_abandoned(
            InjectionOutcome::Abandoned,
            Some(&abandoned),
            "run-abandoned",
        );
        assert_eq!(abandoned_releases.load(Ordering::SeqCst), 1);

        let failed_releases = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let failed_counter = failed_releases.clone();
        let failed = OnInjected::new(
            || anyhow::bail!("arm failed"),
            || anyhow::bail!("settle failed"),
        )
        .with_process_dispatch_release(move || {
            failed_counter.fetch_add(1, Ordering::SeqCst);
        });
        assert!(failed.arm_no_replay().is_err());
        assert!(failed.settle().is_err());
        assert_eq!(
            failed_releases.load(Ordering::SeqCst),
            0,
            "a failed durable callback must keep the process claim pinned against the next sweep"
        );

        // Keep the queued receipt alive through the end of the assertions; its
        // plain Drop is intentionally not a release event.
        drop(queued);
        assert_eq!(queued_releases.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn primary_handoff_requires_a_durable_replay_receipt() {
        assert!(
            !can_defer_to_primary(None),
            "receipt-free Group/hook callbacks have no Primary replay source"
        );
        let process_local = OnInjected::idempotent(|| Ok(()));
        assert!(
            !can_defer_to_primary(Some(&process_local)),
            "a process notification callback is not a durable handoff"
        );
        let default_receipt = OnInjected::new(|| Ok(()), || Ok(()));
        assert!(
            !can_defer_to_primary(Some(&default_receipt)),
            "durable storage alone does not imply periodic Primary discovery"
        );
        let periodic_source = OnInjected::new(|| Ok(()), || Ok(())).with_primary_handoff();
        assert!(can_defer_to_primary(Some(&periodic_source)));
    }

    #[test]
    fn pending_flush_claims_one_and_preserves_same_session_fifo() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Arc::new(crate::session::SessionDB::open(&tmp.path().join("s.db")).unwrap());
        let pending = |session: &str, run: &str| PendingInjection {
            parent_session_id: session.to_string(),
            parent_agent_id: "ha-main".to_string(),
            child_agent_id: "helper".to_string(),
            run_id: run.to_string(),
            push_message: "done".to_string(),
            session_db: db.clone(),
            on_injected: None,
            reattachable_ui_guard: None,
            gate: PendingInjectionGate::Ready,
        };
        let mut queue = vec![
            pending("s1", "run-1"),
            pending("s2", "run-2"),
            pending("s1", "run-3"),
        ];
        let mut injecting = std::collections::HashMap::new();

        assert_eq!(
            claim_next_pending_injection(&mut queue, &mut injecting, "s1")
                .unwrap()
                .run_id,
            "run-1"
        );
        assert_eq!(
            queue
                .iter()
                .map(|task| task.run_id.as_str())
                .collect::<Vec<_>>(),
            ["run-2", "run-3"]
        );
        assert!(
            claim_next_pending_injection(&mut queue, &mut injecting, "s1").is_none(),
            "a concurrent flush must not dequeue the same-session suffix"
        );
        assert_eq!(
            queue
                .iter()
                .map(|task| task.run_id.as_str())
                .collect::<Vec<_>>(),
            ["run-2", "run-3"]
        );
        injecting.remove("s1");
        assert_eq!(
            claim_next_pending_injection(&mut queue, &mut injecting, "s1")
                .unwrap()
                .run_id,
            "run-3"
        );
        assert_eq!(queue[0].run_id, "run-2");
    }

    #[test]
    fn unified_fifo_never_bypasses_a_blocked_head() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Arc::new(crate::session::SessionDB::open(&tmp.path().join("s.db")).unwrap());
        let task = |session_id: &str, run_id: &str, gate: PendingInjectionGate| PendingInjection {
            parent_session_id: session_id.to_string(),
            parent_agent_id: "ha-main".to_string(),
            child_agent_id: "helper".to_string(),
            run_id: run_id.to_string(),
            push_message: "done".to_string(),
            session_db: db.clone(),
            on_injected: None,
            reattachable_ui_guard: None,
            gate,
        };
        let mut durable_task = task(
            "session-a",
            "run-a",
            PendingInjectionGate::Channel {
                account_id: Some("account-a".to_string()),
            },
        );
        durable_task.on_injected = Some(OnInjected::new(|| Ok(()), || Ok(())));
        let mut pending = vec![
            durable_task,
            task("session-a", "run-b", PendingInjectionGate::Ready),
            task(
                "session-b",
                "run-c",
                PendingInjectionGate::Channel {
                    account_id: Some("account-b".to_string()),
                },
            ),
            task(
                "session-unknown",
                "run-unknown",
                PendingInjectionGate::Channel { account_id: None },
            ),
        ];
        let mut injecting = std::collections::HashMap::new();

        assert!(
            claim_next_pending_injection(&mut pending, &mut injecting, "session-a").is_none(),
            "ready B must not bypass channel-blocked A"
        );

        let opened = open_channel_gates(&mut pending, Some("account-b"));
        assert!(opened.contains(&"session-b".to_string()));
        assert!(opened.contains(&"session-unknown".to_string()));
        assert!(
            claim_next_pending_injection(&mut pending, &mut injecting, "session-a").is_none(),
            "an unrelated account event must leave A blocked"
        );

        let opened = open_channel_gates(&mut pending, Some("account-a"));
        assert_eq!(opened, ["session-a".to_string()]);
        let first = claim_next_pending_injection(&mut pending, &mut injecting, "session-a")
            .expect("account A event opens FIFO head");
        assert_eq!(first.run_id, "run-a");
        assert!(first.on_injected.is_some(), "receipt must stay with A");
        injecting.remove("session-a");
        assert_eq!(
            claim_next_pending_injection(&mut pending, &mut injecting, "session-a")
                .unwrap()
                .run_id,
            "run-b"
        );
    }

    #[test]
    fn active_retry_returns_to_same_session_head_and_new_work_dedups_at_tail() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Arc::new(crate::session::SessionDB::open(&tmp.path().join("s.db")).unwrap());
        let task = |session: &str, run: &str, gate: PendingInjectionGate| PendingInjection {
            parent_session_id: session.to_string(),
            parent_agent_id: "ha-main".to_string(),
            child_agent_id: "helper".to_string(),
            run_id: run.to_string(),
            push_message: "done".to_string(),
            session_db: db.clone(),
            on_injected: None,
            reattachable_ui_guard: None,
            gate,
        };
        let mut queue = vec![
            task("other", "x", PendingInjectionGate::Ready),
            task("session", "b", PendingInjectionGate::Ready),
            task("session", "c", PendingInjectionGate::Ready),
        ];
        enqueue_active_retry_front(
            &mut queue,
            task(
                "session",
                "a",
                PendingInjectionGate::Channel {
                    account_id: Some("account".to_string()),
                },
            ),
        );
        assert_eq!(
            queue
                .iter()
                .filter(|task| task.parent_session_id == "session")
                .map(|task| task.run_id.as_str())
                .collect::<Vec<_>>(),
            ["a", "b", "c"]
        );

        enqueue_new_pending_tail(
            &mut queue,
            task("session", "b", PendingInjectionGate::Ready),
        );
        enqueue_new_pending_tail(
            &mut queue,
            task("session", "d", PendingInjectionGate::Ready),
        );
        assert_eq!(
            queue
                .iter()
                .filter(|task| task.parent_session_id == "session")
                .map(|task| task.run_id.as_str())
                .collect::<Vec<_>>(),
            ["a", "b", "c", "d"]
        );
    }

    #[test]
    fn active_identity_duplicate_dispatch_coalesces_without_entering_retry_tail() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Arc::new(crate::session::SessionDB::open(&tmp.path().join("s.db")).unwrap());
        let task = |run: &str| PendingInjection {
            parent_session_id: "session".to_string(),
            parent_agent_id: "ha-main".to_string(),
            child_agent_id: "helper".to_string(),
            run_id: run.to_string(),
            push_message: "done".to_string(),
            session_db: db.clone(),
            on_injected: None,
            reattachable_ui_guard: None,
            gate: PendingInjectionGate::Ready,
        };
        let mut injecting =
            std::collections::HashMap::from([("session".to_string(), "run-a".to_string())]);
        let mut queue = vec![task("run-b")];

        assert_eq!(
            admit_or_enqueue_injection(&mut injecting, &mut queue, &task("run-a")),
            InjectionAdmission::Coalesced
        );
        assert_eq!(
            queue
                .iter()
                .map(|pending| pending.run_id.as_str())
                .collect::<Vec<_>>(),
            ["run-b"],
            "a duplicate sweep for active retry A must not enter the FIFO tail"
        );

        assert_eq!(
            admit_or_enqueue_injection(&mut injecting, &mut queue, &task("run-c")),
            InjectionAdmission::Queued
        );
        assert_eq!(
            queue
                .iter()
                .map(|pending| pending.run_id.as_str())
                .collect::<Vec<_>>(),
            ["run-b", "run-c"],
            "a distinct run still preserves FIFO admission"
        );
    }

    #[test]
    fn cleanup_releases_only_its_exact_active_owner() {
        let mut injecting =
            std::collections::HashMap::from([("session".to_string(), "new-owner".to_string())]);

        assert!(!release_injection_owner(
            &mut injecting,
            "session",
            "stale-owner"
        ));
        assert_eq!(
            injecting.get("session").map(String::as_str),
            Some("new-owner")
        );
        assert!(release_injection_owner(
            &mut injecting,
            "session",
            "new-owner"
        ));
        assert!(!injecting.contains_key("session"));
    }

    #[test]
    fn purge_removes_ready_and_channel_gated_tasks_for_only_one_session() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Arc::new(crate::session::SessionDB::open(&tmp.path().join("s.db")).unwrap());
        let task = |session: &str, run: &str, gate: PendingInjectionGate| PendingInjection {
            parent_session_id: session.to_string(),
            parent_agent_id: "ha-main".to_string(),
            child_agent_id: "helper".to_string(),
            run_id: run.to_string(),
            push_message: "done".to_string(),
            session_db: db.clone(),
            on_injected: None,
            reattachable_ui_guard: None,
            gate,
        };
        let process_releases = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let process_releases_for_receipt = process_releases.clone();
        let mut claimed = task("gone", "ready", PendingInjectionGate::Ready);
        claimed.on_injected = Some(
            OnInjected::new(|| Ok(()), || Ok(())).with_process_dispatch_release(move || {
                process_releases_for_receipt.fetch_add(1, Ordering::SeqCst);
            }),
        );
        let mut queue = vec![
            claimed,
            task(
                "gone",
                "blocked",
                PendingInjectionGate::Channel {
                    account_id: Some("account".to_string()),
                },
            ),
            task("keep", "ready", PendingInjectionGate::Ready),
        ];

        assert_eq!(purge_pending_from_queue(&mut queue, "gone"), 2);
        assert_eq!(queue.len(), 1);
        assert_eq!(queue[0].parent_session_id, "keep");
        assert_eq!(
            process_releases.load(Ordering::SeqCst),
            1,
            "session cleanup must not leak a queued async-job dispatch claim"
        );
    }

    #[test]
    fn enqueue_recheck_opens_running_removed_or_disabled_surfaces() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Arc::new(crate::session::SessionDB::open(&tmp.path().join("s.db")).unwrap());
        for (case, persisted_enabled, running, should_open) in [
            ("running", Some(true), true, true),
            ("disabled", Some(false), false, true),
            ("removed", None, false, true),
            ("still-starting", Some(true), false, false),
        ] {
            let mut queue = vec![PendingInjection {
                parent_session_id: format!("session-{case}"),
                parent_agent_id: "ha-main".to_string(),
                child_agent_id: "helper".to_string(),
                run_id: format!("run-{case}"),
                push_message: "done".to_string(),
                session_db: db.clone(),
                on_injected: None,
                reattachable_ui_guard: None,
                gate: PendingInjectionGate::Channel {
                    account_id: Some("account".to_string()),
                },
            }];
            if persisted_surface_releases_channel_gate(persisted_enabled, running) {
                open_channel_gates(&mut queue, Some("account"));
            }
            let session_id = queue[0].parent_session_id.clone();
            let mut injecting = std::collections::HashMap::new();
            assert_eq!(
                claim_next_pending_injection(&mut queue, &mut injecting, &session_id).is_some(),
                should_open,
                "event-before-enqueue recheck case: {case}"
            );
        }
    }

    // R2 (§5.4): the idle gate must park completion injection behind a live
    // foreground turn on *every* entry point. These exercise the shared wait
    // helper against `ChatSessionGuard` (the same guard `run_chat_engine` now
    // creates for HTTP / IM / cron, and ACP creates at its turn boundary).

    #[tokio::test]
    async fn wait_for_session_idle_parks_until_guard_released() {
        let sid = "test-r2-wait-idle-parks";
        crate::subagent::ACTIVE_CHAT_SESSIONS
            .lock()
            .unwrap()
            .remove(sid);

        // A live foreground turn holds the guard → busy → a bounded wait times
        // out rather than firing (injection would NOT splice into a live turn).
        let guard = crate::subagent::ChatSessionGuard::new(sid);
        let outcome =
            wait_for_session_idle(sid, std::time::Duration::from_millis(120), || false).await;
        assert!(matches!(outcome, IdleWait::TimedOut));

        // Releasing the turn makes the session idle → the next wait returns Idle.
        drop(guard);
        let outcome = wait_for_session_idle(sid, std::time::Duration::from_secs(2), || false).await;
        assert!(matches!(outcome, IdleWait::Idle));
    }

    #[tokio::test]
    async fn wait_for_session_idle_aborts_when_should_abort_fires() {
        let sid = "test-r2-wait-idle-abort";
        crate::subagent::ACTIVE_CHAT_SESSIONS
            .lock()
            .unwrap()
            .remove(sid);

        // Busy, but the agent already fetched the result → Aborted (caller
        // fires on_injected and returns Injected without running a turn).
        let _guard = crate::subagent::ChatSessionGuard::new(sid);
        let outcome = wait_for_session_idle(sid, std::time::Duration::from_secs(2), || true).await;
        assert!(matches!(outcome, IdleWait::Aborted));
    }

    #[tokio::test]
    async fn wait_for_session_idle_idle_when_no_turn_active() {
        let sid = "test-r2-wait-idle-noturn";
        crate::subagent::ACTIVE_CHAT_SESSIONS
            .lock()
            .unwrap()
            .remove(sid);
        let outcome = wait_for_session_idle(sid, std::time::Duration::from_secs(2), || false).await;
        assert!(matches!(outcome, IdleWait::Idle));
    }
}
