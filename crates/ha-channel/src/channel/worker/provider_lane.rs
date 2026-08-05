//! Process-local ordering for non-idempotent provider mutations that target
//! the same physical IM conversation.
//!
//! Reservation is synchronous, so order follows pipeline/catch-up creation
//! rather than whichever Tokio task happens to be polled first.  A stream task
//! and its outer [`ProviderLaneLease`] share the current node; the next turn is
//! released only after both streaming I/O and terminal delivery have ended.

use std::collections::HashMap;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use futures_util::FutureExt;
use tokio::sync::{mpsc, oneshot, watch};

use super::pipeline::DeliveryTarget;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct ProviderTargetKey {
    account_id: String,
    chat_id: String,
    thread_id: Option<String>,
}

impl ProviderTargetKey {
    fn from_target(target: &DeliveryTarget<'_>) -> Self {
        Self {
            account_id: target.account_id.to_string(),
            chat_id: target.chat_id.to_string(),
            thread_id: target.thread_id.map(str::to_string),
        }
    }
}

struct ProviderLaneNode {
    key: ProviderTargetKey,
    completed: watch::Sender<bool>,
    predecessor_completed: AtomicBool,
    owner_released: AtomicBool,
    completion_published: AtomicBool,
    // The predecessor owns its immediate successor until it publishes
    // completion.  This keeps a cancelled queued reservation in the chain:
    // otherwise dropping B before A completed would remove B from the tail
    // and let a newly reserved C overtake A.
    successor: Mutex<Option<Arc<ProviderLaneNode>>>,
}

struct ProviderLaneOwner {
    node: Arc<ProviderLaneNode>,
}

type ProviderLaneMap = HashMap<ProviderTargetKey, Weak<ProviderLaneNode>>;

static PROVIDER_LANES: OnceLock<Mutex<ProviderLaneMap>> = OnceLock::new();

type ProviderMutationJob = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

struct ProviderMutationExecutor {
    sender: mpsc::UnboundedSender<ProviderMutationJob>,
}

static PROVIDER_MUTATION_EXECUTOR: OnceLock<ProviderMutationExecutor> = OnceLock::new();

fn provider_lanes() -> &'static Mutex<ProviderLaneMap> {
    PROVIDER_LANES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn provider_mutation_executor() -> &'static ProviderMutationExecutor {
    PROVIDER_MUTATION_EXECUTOR.get_or_init(|| {
        let (sender, mut receiver) = mpsc::unbounded_channel::<ProviderMutationJob>();
        let spawn_result = std::thread::Builder::new()
            .name("ha-provider-mutations".to_string())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        app_error!(
                            "channel",
                            "provider_lane",
                            "Unable to start provider mutation runtime: {}",
                            error
                        );
                        return;
                    }
                };
                runtime.block_on(async move {
                    while let Some(job) = receiver.recv().await {
                        // Jobs must be independent: a slow physical target
                        // cannot stall dispatch for every other target.
                        tokio::spawn(job);
                    }
                });
            });
        if let Err(error) = spawn_result {
            app_error!(
                "channel",
                "provider_lane",
                "Unable to spawn provider mutation thread: {}",
                error
            );
        }
        ProviderMutationExecutor { sender }
    })
}

/// Run provider-adjacent orchestration on the same process-lifetime runtime
/// used by guarded mutations. Unlike `tokio::spawn` at a request site, this
/// task is not owned by a short-lived caller runtime. The task must still use
/// [`ProviderMutationGuard`] for every actual provider write.
pub(crate) fn spawn_provider_process_task<Fut>(task: Fut)
where
    Fut: Future<Output = ()> + Send + 'static,
{
    let job = Box::pin(async move {
        if AssertUnwindSafe(task).catch_unwind().await.is_err() {
            app_warn!(
                "channel",
                "provider_lane",
                "Provider background task panicked"
            );
        }
    });
    if provider_mutation_executor().sender.send(job).is_err() {
        app_error!(
            "channel",
            "provider_lane",
            "Provider mutation executor is unavailable for background task"
        );
    }
}

/// A clonable wait view handed to the stream task.  It only observes the
/// predecessor and cannot complete the current reservation.
#[derive(Clone)]
pub(crate) struct ProviderLaneWaiter {
    predecessor: Option<watch::Receiver<bool>>,
}

impl ProviderLaneWaiter {
    pub(crate) async fn wait_turn(&self) {
        let Some(mut predecessor) = self.predecessor.clone() else {
            return;
        };
        while !*predecessor.borrow() {
            if predecessor.changed().await.is_err() {
                // The predecessor sender can disappear only after its owning
                // lease was dropped, which is the same completion boundary.
                break;
            }
        }
    }
}

/// Exclusive place in one physical target's provider-mutation sequence.
/// The current generation completes after its last owner drops and its own
/// predecessor has completed; a cancelled queued generation therefore stays
/// as a transitive barrier instead of opening a hole in reservation order.
pub(crate) struct ProviderLaneLease {
    owner: Arc<ProviderLaneOwner>,
    waiter: ProviderLaneWaiter,
}

impl ProviderLaneLease {
    pub(crate) fn waiter(&self) -> ProviderLaneWaiter {
        self.waiter.clone()
    }

    pub(crate) fn task_hold(&self) -> ProviderLaneTaskHold {
        ProviderLaneTaskHold {
            _owner: self.owner.clone(),
        }
    }
}

/// The stream task's ownership share.  Keeping this separate from the waiter
/// makes it impossible for waiting alone to release the current reservation.
#[derive(Clone)]
pub(crate) struct ProviderLaneTaskHold {
    _owner: Arc<ProviderLaneOwner>,
}

/// A lane owner plus the live authorization for its original physical target.
/// Every provider mutation is submitted to the process-lifetime executor with
/// a clone of this guard. Therefore dropping the caller future—or an entire
/// short-lived Tokio runtime—cannot release the lane while the remote request
/// may still settle.
#[derive(Clone)]
pub(crate) struct ProviderMutationGuard {
    _lane_hold: ProviderLaneTaskHold,
    lane_waiter: ProviderLaneWaiter,
    still_valid: Arc<dyn Fn() -> bool + Send + Sync>,
}

pub(crate) enum ProviderMutationOutcome<T> {
    Completed(T),
    Invalid,
    TaskFailed,
}

pub(crate) struct ProviderMutationTicket<T> {
    receiver: Option<oneshot::Receiver<ProviderMutationOutcome<T>>>,
}

struct ResilientMutationState<T> {
    outcome: Mutex<Option<ProviderMutationOutcome<T>>>,
    abandoned: AtomicBool,
    notify: tokio::sync::Notify,
}

pub(crate) struct ResilientProviderMutationTicket<T> {
    state: Arc<ResilientMutationState<T>>,
    consumed: bool,
}

impl<T> ResilientProviderMutationTicket<T> {
    pub(crate) async fn wait(mut self) -> ProviderMutationOutcome<T> {
        loop {
            let notified = self.state.notify.notified();
            tokio::pin!(notified);
            // `notify_waiters` does not retain a permit for a Notified future
            // that has not registered yet. Register before inspecting the
            // shared outcome so a fast provider completion cannot land in the
            // check -> first-poll gap and strand this ticket forever.
            notified.as_mut().enable();
            if let Some(outcome) = self
                .state
                .outcome
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take()
            {
                self.consumed = true;
                self.state.notify.notify_waiters();
                return outcome;
            }
            notified.as_mut().await;
        }
    }
}

impl<T> Drop for ResilientProviderMutationTicket<T> {
    fn drop(&mut self) {
        if !self.consumed {
            self.state.abandoned.store(true, Ordering::Release);
            self.state.notify.notify_waiters();
        }
    }
}

impl<T> ProviderMutationTicket<T> {
    pub(crate) async fn wait(mut self) -> ProviderMutationOutcome<T> {
        let Some(receiver) = self.receiver.take() else {
            return ProviderMutationOutcome::TaskFailed;
        };
        receiver
            .await
            .unwrap_or(ProviderMutationOutcome::TaskFailed)
    }
}

impl ProviderMutationGuard {
    pub(crate) fn new(
        lane_waiter: ProviderLaneWaiter,
        lane_hold: ProviderLaneTaskHold,
        still_valid: Arc<dyn Fn() -> bool + Send + Sync>,
    ) -> Self {
        Self {
            _lane_hold: lane_hold,
            lane_waiter,
            still_valid,
        }
    }

    pub(crate) fn is_valid(&self) -> bool {
        (self.still_valid)()
    }

    /// Evaluate live target validity away from the async runtime thread.
    /// Mirror validity normally reads the ChannelDB; callers on the stream or
    /// process provider executor must not turn that synchronous SQLite lookup
    /// into head-of-line blocking for GUI deltas or unrelated IM targets.
    pub(crate) async fn is_valid_async(&self) -> bool {
        let guard = self.clone();
        tokio::task::spawn_blocking(move || guard.is_valid())
            .await
            .unwrap_or(false)
    }

    pub(crate) fn submit<T, Fut>(&self, mutation: Fut) -> ProviderMutationTicket<T>
    where
        T: Send + 'static,
        Fut: Future<Output = T> + Send + 'static,
    {
        let guard = self.clone();
        let (result_tx, result_rx) = oneshot::channel();
        let job = Box::pin(async move {
            // This guard is also used by cancel/error paths that can race the
            // prelude itself. Enforce the reservation barrier here rather
            // than trusting every caller to have observed it already.
            guard.lane_waiter.wait_turn().await;
            // Mirror validity may consult the channel SQLite registry. Keep
            // that synchronous read off the dedicated async executor thread,
            // so one contended DB cannot stall unrelated physical targets.
            let valid = guard.is_valid_async().await;
            let outcome = if !valid {
                ProviderMutationOutcome::Invalid
            } else {
                match AssertUnwindSafe(mutation).catch_unwind().await {
                    Ok(result) => ProviderMutationOutcome::Completed(result),
                    Err(_) => {
                        app_warn!(
                            "channel",
                            "provider_lane",
                            "Provider mutation task panicked; outcome is ambiguous"
                        );
                        ProviderMutationOutcome::TaskFailed
                    }
                }
            };
            // A dropped receiver means its caller/runtime went away. The job
            // has nevertheless reached a terminal provider outcome while
            // retaining `guard`, which is the safety property we need.
            let _ = result_tx.send(outcome);
        });
        if provider_mutation_executor().sender.send(job).is_err() {
            app_error!(
                "channel",
                "provider_lane",
                "Provider mutation executor is unavailable"
            );
            return ProviderMutationTicket { receiver: None };
        }
        ProviderMutationTicket {
            receiver: Some(result_rx),
        }
    }

    pub(crate) async fn run<T, Fut>(&self, mutation: Fut) -> ProviderMutationOutcome<T>
    where
        T: Send + 'static,
        Fut: Future<Output = T> + Send + 'static,
    {
        self.submit(mutation).wait().await
    }

    /// Process-lifetime cleanup for a provider handle that was already
    /// created while the target was valid. This deliberately skips the live
    /// attach check so a moved binding can still close its old persistent
    /// stream. It must never be used for a new standalone send/open.
    pub(crate) fn submit_cleanup<T, Fut>(&self, cleanup: Fut) -> ProviderMutationTicket<T>
    where
        T: Send + 'static,
        Fut: Future<Output = T> + Send + 'static,
    {
        let guard = self.clone();
        let (result_tx, result_rx) = oneshot::channel();
        let job = Box::pin(async move {
            guard.lane_waiter.wait_turn().await;
            let outcome = match AssertUnwindSafe(cleanup).catch_unwind().await {
                Ok(result) => ProviderMutationOutcome::Completed(result),
                Err(_) => ProviderMutationOutcome::TaskFailed,
            };
            let _ = result_tx.send(outcome);
        });
        if provider_mutation_executor().sender.send(job).is_err() {
            return ProviderMutationTicket { receiver: None };
        }
        ProviderMutationTicket {
            receiver: Some(result_rx),
        }
    }

    /// Submit a mutation whose successful value itself owns remote lifecycle
    /// state (for example an opened native reply stream). If the caller
    /// disappears while the new mutation is still queued, it is never polled.
    /// Once the mutation has started, an unconsumed successful value runs
    /// `cleanup` on the process-lifetime executor before this guard releases
    /// the lane.
    pub(crate) fn submit_resilient<T, Fut, Cleanup, CleanupFut>(
        &self,
        mutation: Fut,
        cleanup: Cleanup,
    ) -> ResilientProviderMutationTicket<T>
    where
        T: Send + 'static,
        Fut: Future<Output = T> + Send + 'static,
        Cleanup: FnOnce(T) -> CleanupFut + Send + 'static,
        CleanupFut: Future<Output = ()> + Send + 'static,
    {
        self.submit_resilient_inner(mutation, cleanup, true)
    }

    /// Resilient variant for a mutation that already owns an existing remote
    /// handle. The mutation itself must choose normal use vs detached cleanup;
    /// the executor only enforces the lane barrier and abandoned-value cleanup.
    pub(crate) fn submit_resilient_cleanup<T, Fut, Cleanup, CleanupFut>(
        &self,
        mutation: Fut,
        cleanup: Cleanup,
    ) -> ResilientProviderMutationTicket<T>
    where
        T: Send + 'static,
        Fut: Future<Output = T> + Send + 'static,
        Cleanup: FnOnce(T) -> CleanupFut + Send + 'static,
        CleanupFut: Future<Output = ()> + Send + 'static,
    {
        self.submit_resilient_inner(mutation, cleanup, false)
    }

    fn submit_resilient_inner<T, Fut, Cleanup, CleanupFut>(
        &self,
        mutation: Fut,
        cleanup: Cleanup,
        validate_target: bool,
    ) -> ResilientProviderMutationTicket<T>
    where
        T: Send + 'static,
        Fut: Future<Output = T> + Send + 'static,
        Cleanup: FnOnce(T) -> CleanupFut + Send + 'static,
        CleanupFut: Future<Output = ()> + Send + 'static,
    {
        let guard = self.clone();
        let state = Arc::new(ResilientMutationState {
            outcome: Mutex::new(None),
            abandoned: AtomicBool::new(false),
            notify: tokio::sync::Notify::new(),
        });
        let job_state = state.clone();
        let job = Box::pin(async move {
            guard.lane_waiter.wait_turn().await;
            let valid = if validate_target {
                guard.is_valid_async().await
            } else {
                true
            };
            // A resilient new mutation only protects a provider request that
            // has already started. If its caller disappeared while this job
            // was queued behind a predecessor (or while validity was being
            // checked), there is no result/handle to rescue: starting the
            // mutation now would create a new visible side effect after
            // cancellation. Existing-handle cleanup deliberately bypasses
            // this branch so an accepted stream is never abandoned without
            // its abort/close.
            if validate_target && job_state.abandoned.load(Ordering::Acquire) {
                return;
            }
            let outcome = if !valid {
                ProviderMutationOutcome::Invalid
            } else {
                match AssertUnwindSafe(mutation).catch_unwind().await {
                    Ok(result) => ProviderMutationOutcome::Completed(result),
                    Err(_) => ProviderMutationOutcome::TaskFailed,
                }
            };

            if job_state.abandoned.load(Ordering::Acquire) {
                if let ProviderMutationOutcome::Completed(value) = outcome {
                    cleanup(value).await;
                }
                return;
            }

            *job_state
                .outcome
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(outcome);
            job_state.notify.notify_waiters();

            loop {
                let notified = job_state.notify.notified();
                tokio::pin!(notified);
                // The ticket can consume/drop its outcome from another runtime
                // thread. Register first so that acknowledgement cannot be
                // lost between these state checks and the initial await.
                notified.as_mut().enable();
                if job_state.abandoned.load(Ordering::Acquire) {
                    let abandoned = job_state
                        .outcome
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .take();
                    if let Some(ProviderMutationOutcome::Completed(value)) = abandoned {
                        cleanup(value).await;
                    }
                    return;
                }
                if job_state
                    .outcome
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .is_none()
                {
                    return;
                }
                notified.as_mut().await;
            }
        });

        if provider_mutation_executor().sender.send(job).is_err() {
            *state
                .outcome
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) =
                Some(ProviderMutationOutcome::TaskFailed);
            state.notify.notify_waiters();
        }
        ResilientProviderMutationTicket {
            state,
            consumed: false,
        }
    }
}

impl Drop for ProviderLaneOwner {
    fn drop(&mut self) {
        release_provider_lane_node(self.node.clone());
    }
}

fn release_provider_lane_node(node: Arc<ProviderLaneNode>) {
    node.owner_released.store(true, Ordering::Release);
    let mut lanes = provider_lanes()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut current = Some(node);

    // Complete a cancelled queued suffix transitively once its live
    // predecessor settles. All link and tail transitions happen under the
    // lane-map lock, matching reservation and avoiding a completion-vs-link
    // race at the physical target's current tail.
    while let Some(node) = current.take() {
        if !node.predecessor_completed.load(Ordering::Acquire)
            || !node.owner_released.load(Ordering::Acquire)
            || node.completion_published.swap(true, Ordering::AcqRel)
        {
            break;
        }

        node.completed.send_replace(true);
        let node_ptr = Arc::as_ptr(&node);
        if lanes
            .get(&node.key)
            .is_some_and(|tail| tail.as_ptr() == node_ptr)
        {
            lanes.remove(&node.key);
        }

        let successor = node
            .successor
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        if let Some(successor) = successor {
            successor
                .predecessor_completed
                .store(true, Ordering::Release);
            current = Some(successor);
        }
    }
}

pub(crate) fn reserve_provider_lane(target: &DeliveryTarget<'_>) -> ProviderLaneLease {
    let key = ProviderTargetKey::from_target(target);
    let (completed, _receiver) = watch::channel(false);
    let node = Arc::new(ProviderLaneNode {
        key: key.clone(),
        completed,
        predecessor_completed: AtomicBool::new(false),
        owner_released: AtomicBool::new(false),
        completion_published: AtomicBool::new(false),
        successor: Mutex::new(None),
    });
    let mut lanes = provider_lanes()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let predecessor = lanes.get(&key).and_then(Weak::upgrade);
    if let Some(predecessor) = predecessor.as_ref() {
        let replaced = predecessor
            .successor
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .replace(node.clone());
        debug_assert!(
            replaced.is_none(),
            "provider lane tail already had a successor"
        );
    } else {
        node.predecessor_completed.store(true, Ordering::Release);
    }
    lanes.insert(key.clone(), Arc::downgrade(&node));
    drop(lanes);

    // Capture the predecessor at reservation time.  Looking it up later would
    // see the current tail and lose the creation-order relationship.
    let waiter = ProviderLaneWaiter {
        predecessor: predecessor.map(|node| node.completed.subscribe()),
    };
    ProviderLaneLease {
        owner: Arc::new(ProviderLaneOwner { node }),
        waiter,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::FutureExt;
    use ha_core::channel::types::ChatType;

    fn reserve(account_id: &str, chat_id: &str, thread_id: Option<&str>) -> ProviderLaneLease {
        reserve_provider_lane(&DeliveryTarget {
            account_id,
            chat_id,
            chat_type: &ChatType::Dm,
            thread_id,
            reply_to_message_id: None,
            recipient_user_id: None,
            recipient_tenant_id: None,
        })
    }

    #[tokio::test]
    async fn provider_lane_uses_reservation_order_even_when_successor_polls_first() {
        let chat_id = format!("lane-order-{}", uuid::Uuid::new_v4());
        let first = reserve("account", &chat_id, Some("thread"));
        let second = reserve("account", &chat_id, Some("thread"));
        let second_waiter = second.waiter();
        let mut second_wait = Box::pin(second_waiter.wait_turn());

        assert!(second_wait.as_mut().now_or_never().is_none());
        let first_waiter = first.waiter();
        assert!(Box::pin(first_waiter.wait_turn())
            .as_mut()
            .now_or_never()
            .is_some());

        drop(first);
        tokio::time::timeout(std::time::Duration::from_secs(1), second_wait)
            .await
            .expect("successor should enter after predecessor release");
        drop(second);
    }

    #[tokio::test]
    async fn provider_lane_waits_for_outer_terminal_after_stream_task_finishes() {
        let chat_id = format!("lane-terminal-{}", uuid::Uuid::new_v4());
        let outer_terminal = reserve("account", &chat_id, None);
        let stream_task_hold = outer_terminal.task_hold();
        let successor = reserve("account", &chat_id, None);
        let successor_waiter = successor.waiter();

        // Stream EOF releases only the task's share.  The final/abort owner is
        // still performing its terminal provider mutation.
        drop(stream_task_hold);
        let mut successor_wait = Box::pin(successor_waiter.wait_turn());
        assert!(successor_wait.as_mut().now_or_never().is_none());

        drop(outer_terminal);
        tokio::time::timeout(std::time::Duration::from_secs(1), successor_wait)
            .await
            .expect("outer terminal release should unblock successor");
        drop(successor);
    }

    #[tokio::test]
    async fn provider_lane_detached_terminal_hold_survives_outer_drop() {
        let chat_id = format!("lane-detached-terminal-{}", uuid::Uuid::new_v4());
        let outer = reserve("account", &chat_id, None);
        let detached_terminal_hold = outer.task_hold();
        let successor = reserve("account", &chat_id, None);
        let successor_waiter = successor.waiter();
        let mut successor_wait = Box::pin(successor_waiter.wait_turn());

        drop(outer);
        assert!(successor_wait.as_mut().now_or_never().is_none());

        drop(detached_terminal_hold);
        tokio::time::timeout(std::time::Duration::from_secs(1), successor_wait)
            .await
            .expect("detached provider terminal completion should release successor");
        drop(successor);
    }

    #[tokio::test]
    async fn provider_lane_keeps_different_physical_targets_parallel() {
        let suffix = uuid::Uuid::new_v4();
        let blocked_target = reserve("account", &format!("chat-a-{suffix}"), None);
        let independent_target = reserve("account", &format!("chat-b-{suffix}"), None);

        let independent_waiter = independent_target.waiter();
        assert!(Box::pin(independent_waiter.wait_turn())
            .as_mut()
            .now_or_never()
            .is_some());

        drop(blocked_target);
        drop(independent_target);
    }

    #[tokio::test]
    async fn cancelled_queued_reservation_cannot_let_a_successor_overtake() {
        let chat_id = format!("lane-cancelled-middle-{}", uuid::Uuid::new_v4());
        let first = reserve("account", &chat_id, None);
        let cancelled_middle = reserve("account", &chat_id, None);

        // Cancelling B while it still waits for A must leave a transitive
        // barrier in the chain. A newly-created C remains behind A even
        // though no live task owns B anymore.
        drop(cancelled_middle);
        let third = reserve("account", &chat_id, None);
        let third_waiter = third.waiter();
        let mut third_wait = Box::pin(third_waiter.wait_turn());
        assert!(third_wait.as_mut().now_or_never().is_none());

        drop(first);
        tokio::time::timeout(std::time::Duration::from_secs(1), third_wait)
            .await
            .expect("cancelled middle generation should release only after predecessor");
        drop(third);
    }

    #[tokio::test]
    async fn provider_mutation_survives_caller_runtime_drop_until_remote_settles() {
        let chat_id = format!("lane-runtime-drop-{}", uuid::Uuid::new_v4());
        let lane = reserve("account", &chat_id, None);
        let guard = ProviderMutationGuard::new(lane.waiter(), lane.task_hold(), Arc::new(|| true));
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (finish_tx, finish_rx) = tokio::sync::oneshot::channel();

        // Submit from an explicitly short-lived runtime, then drop both the
        // returned ticket and that runtime. The process executor must retain
        // the mutation and its lane owner until the remote future settles.
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("temporary caller runtime");
            runtime.block_on(async move {
                drop(guard.submit(async move {
                    let _ = started_tx.send(());
                    let _ = finish_rx.await;
                }));
            });
        })
        .join()
        .expect("temporary caller thread");

        drop(lane);
        tokio::time::timeout(std::time::Duration::from_secs(1), started_rx)
            .await
            .expect("process executor should start the accepted mutation")
            .expect("mutation should report start");

        let successor = reserve("account", &chat_id, None);
        let successor_waiter = successor.waiter();
        let mut successor_wait = Box::pin(successor_waiter.wait_turn());
        assert!(successor_wait.as_mut().now_or_never().is_none());

        finish_tx.send(()).expect("release provider mutation");
        tokio::time::timeout(std::time::Duration::from_secs(1), successor_wait)
            .await
            .expect("successor should wait only until the remote mutation settles");
        drop(successor);
    }

    #[tokio::test]
    async fn guard_waits_for_predecessor_and_never_polls_invalid_mutation() {
        let chat_id = format!("lane-invalid-{}", uuid::Uuid::new_v4());
        let predecessor = reserve("account", &chat_id, None);
        let lane = reserve("account", &chat_id, None);
        let guard = ProviderMutationGuard::new(lane.waiter(), lane.task_hold(), Arc::new(|| false));
        let polled = Arc::new(AtomicBool::new(false));
        let mutation_polled = polled.clone();
        let mut outcome = tokio::spawn(async move {
            guard
                .run(async move {
                    mutation_polled.store(true, Ordering::Release);
                })
                .await
        });

        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(30), &mut outcome)
                .await
                .is_err(),
            "guard must enforce the reservation barrier itself"
        );
        assert!(!polled.load(Ordering::Acquire));

        drop(predecessor);
        let result = tokio::time::timeout(std::time::Duration::from_secs(1), outcome)
            .await
            .expect("invalid guard should settle after predecessor")
            .expect("guard task should not panic");
        assert!(matches!(result, ProviderMutationOutcome::Invalid));
        assert!(
            !polled.load(Ordering::Acquire),
            "invalid provider future must never be polled"
        );
        drop(lane);
    }

    #[tokio::test]
    async fn abandoned_queued_new_resilient_mutation_is_never_polled() {
        let chat_id = format!("lane-abandoned-queued-{}", uuid::Uuid::new_v4());
        let predecessor = reserve("account", &chat_id, None);
        let lane = reserve("account", &chat_id, None);
        let validity_checked = Arc::new(AtomicBool::new(false));
        let validity_observer = validity_checked.clone();
        let guard = ProviderMutationGuard::new(
            lane.waiter(),
            lane.task_hold(),
            Arc::new(move || {
                validity_observer.store(true, Ordering::Release);
                true
            }),
        );
        let mutation_polled = Arc::new(AtomicBool::new(false));
        let mutation_observer = mutation_polled.clone();
        let cleanup_polled = Arc::new(AtomicBool::new(false));
        let cleanup_observer = cleanup_polled.clone();
        let ticket = guard.submit_resilient(
            async move {
                mutation_observer.store(true, Ordering::Release);
                42_u8
            },
            move |_| async move {
                cleanup_observer.store(true, Ordering::Release);
            },
        );

        // The caller disappears while its new provider mutation is still
        // queued. Releasing the predecessor must drain the reservation
        // without ever starting that now-unowned side effect.
        drop(ticket);
        drop(guard);
        drop(lane);
        let successor = reserve("account", &chat_id, None);
        let successor_waiter = successor.waiter();
        let mut successor_wait = Box::pin(successor_waiter.wait_turn());
        assert!(successor_wait.as_mut().now_or_never().is_none());

        drop(predecessor);
        tokio::time::timeout(std::time::Duration::from_secs(1), successor_wait)
            .await
            .expect("abandoned queued reservation should drain after its predecessor");
        assert!(
            validity_checked.load(Ordering::Acquire),
            "the abandonment gate must run after live validity"
        );
        assert!(
            !mutation_polled.load(Ordering::Acquire),
            "an abandoned new provider mutation must never be polled"
        );
        assert!(
            !cleanup_polled.load(Ordering::Acquire),
            "there is no accepted value to clean up when mutation never starts"
        );
        drop(successor);
    }

    #[tokio::test]
    async fn abandoned_queued_resilient_cleanup_still_consumes_existing_handle() {
        let chat_id = format!("lane-abandoned-cleanup-{}", uuid::Uuid::new_v4());
        let predecessor = reserve("account", &chat_id, None);
        let lane = reserve("account", &chat_id, None);
        let guard = ProviderMutationGuard::new(lane.waiter(), lane.task_hold(), Arc::new(|| true));
        let mutation_polled = Arc::new(AtomicBool::new(false));
        let mutation_observer = mutation_polled.clone();
        let cleanup_polled = Arc::new(AtomicBool::new(false));
        let cleanup_observer = cleanup_polled.clone();
        let ticket = guard.submit_resilient_cleanup(
            async move {
                mutation_observer.store(true, Ordering::Release);
                42_u8
            },
            move |value| async move {
                assert_eq!(value, 42);
                cleanup_observer.store(true, Ordering::Release);
            },
        );

        drop(ticket);
        drop(guard);
        drop(lane);
        let successor = reserve("account", &chat_id, None);
        let successor_waiter = successor.waiter();
        let successor_wait = successor_waiter.wait_turn();
        drop(predecessor);
        tokio::time::timeout(std::time::Duration::from_secs(1), successor_wait)
            .await
            .expect("existing-handle cleanup should drain after its predecessor");
        assert!(
            mutation_polled.load(Ordering::Acquire),
            "caller abandonment must not skip an existing-handle mutation"
        );
        assert!(
            cleanup_polled.load(Ordering::Acquire),
            "the abandoned cleanup result must still be consumed"
        );
        drop(successor);
    }

    #[tokio::test]
    async fn abandoned_resilient_value_is_cleaned_before_successor_enters() {
        let chat_id = format!("lane-resilient-cleanup-{}", uuid::Uuid::new_v4());
        let lane = reserve("account", &chat_id, None);
        let guard = ProviderMutationGuard::new(lane.waiter(), lane.task_hold(), Arc::new(|| true));
        let (mutation_started_tx, mutation_started_rx) = tokio::sync::oneshot::channel();
        let (mutation_finish_tx, mutation_finish_rx) = tokio::sync::oneshot::channel();
        let (cleanup_started_tx, cleanup_started_rx) = tokio::sync::oneshot::channel();
        let (cleanup_finish_tx, cleanup_finish_rx) = tokio::sync::oneshot::channel();
        let ticket = guard.submit_resilient(
            async move {
                let _ = mutation_started_tx.send(());
                let _ = mutation_finish_rx.await;
                42_u8
            },
            move |value| async move {
                assert_eq!(value, 42);
                let _ = cleanup_started_tx.send(());
                let _ = cleanup_finish_rx.await;
            },
        );

        // Simulate caller/runtime loss after an accepted open/push produced a
        // handle-like value but before the caller could install it.
        tokio::time::timeout(std::time::Duration::from_secs(1), mutation_started_rx)
            .await
            .expect("resilient mutation should start")
            .expect("mutation should report start");
        drop(ticket);
        drop(guard);
        drop(lane);
        mutation_finish_tx.send(()).expect("settle mutation");
        tokio::time::timeout(std::time::Duration::from_secs(1), cleanup_started_rx)
            .await
            .expect("abandoned resilient value should enter cleanup")
            .expect("cleanup should report start");

        let successor = reserve("account", &chat_id, None);
        let successor_waiter = successor.waiter();
        let mut successor_wait = Box::pin(successor_waiter.wait_turn());
        assert!(
            successor_wait.as_mut().now_or_never().is_none(),
            "cleanup must retain the lane after the caller abandons its ticket"
        );

        cleanup_finish_tx.send(()).expect("release cleanup");
        tokio::time::timeout(std::time::Duration::from_secs(1), successor_wait)
            .await
            .expect("successor should enter only after resilient cleanup settles");
        drop(successor);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn fast_resilient_completion_never_loses_ticket_ack() {
        // The provider executor runs on another OS thread. Repeating immediate
        // completions exercises the publication-vs-first-poll window that
        // requires `Notified::enable()` before inspecting shared state.
        for expected in 0_u16..256 {
            let chat_id = format!("lane-fast-resilient-{}-{expected}", uuid::Uuid::new_v4());
            let lane = reserve("account", &chat_id, None);
            let guard =
                ProviderMutationGuard::new(lane.waiter(), lane.task_hold(), Arc::new(|| true));
            let outcome = tokio::time::timeout(
                std::time::Duration::from_secs(1),
                guard
                    .submit_resilient(async move { expected }, |_| async {})
                    .wait(),
            )
            .await
            .expect("fast resilient completion must wake its ticket");
            match outcome {
                ProviderMutationOutcome::Completed(actual) => assert_eq!(actual, expected),
                ProviderMutationOutcome::Invalid | ProviderMutationOutcome::TaskFailed => {
                    panic!("valid resilient mutation did not complete")
                }
            }
            drop(guard);
            drop(lane);
        }
    }
}
