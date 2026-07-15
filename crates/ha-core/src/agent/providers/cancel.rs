use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use super::super::streaming_adapter::RoundOutcome;

const CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(100);

pub(in crate::agent) async fn wait_for_cancel(cancel: &Arc<AtomicBool>) {
    wait_for_cancel_flag(cancel.as_ref()).await;
}

pub(super) async fn wait_for_cancel_flag(cancel: &AtomicBool) {
    loop {
        if cancel.load(Ordering::SeqCst) {
            return;
        }
        tokio::time::sleep(CANCEL_POLL_INTERVAL).await;
    }
}

pub(super) async fn sleep_or_cancel(duration: Duration, cancel: &Arc<AtomicBool>) -> bool {
    if cancel.load(Ordering::SeqCst) {
        return true;
    }
    tokio::select! {
        biased;
        _ = wait_for_cancel(cancel) => true,
        _ = tokio::time::sleep(duration) => cancel.load(Ordering::SeqCst),
    }
}

pub(super) async fn next_chunk_or_cancel<S>(
    stream: &mut S,
    cancel: &Arc<AtomicBool>,
) -> Option<S::Item>
where
    S: futures_util::Stream + Unpin,
{
    next_chunk_or_cancel_flag(stream, cancel.as_ref()).await
}

pub(super) async fn next_chunk_or_cancel_flag<S>(
    stream: &mut S,
    cancel: &AtomicBool,
) -> Option<S::Item>
where
    S: futures_util::Stream + Unpin,
{
    use futures_util::StreamExt;

    if cancel.load(Ordering::SeqCst) {
        return None;
    }
    let next = tokio::select! {
        biased;
        _ = wait_for_cancel_flag(cancel) => return None,
        chunk = stream.next() => chunk,
    };
    if cancel.load(Ordering::SeqCst) {
        return None;
    }
    next
}

pub(super) async fn send_with_cancel(
    request: reqwest::RequestBuilder,
    cancel: &Arc<AtomicBool>,
) -> Result<Option<reqwest::Response>, reqwest::Error> {
    if cancel.load(Ordering::SeqCst) {
        return Ok(None);
    }
    tokio::select! {
        biased;
        _ = wait_for_cancel(cancel) => Ok(None),
        response = request.send() => {
            if cancel.load(Ordering::SeqCst) {
                Ok(None)
            } else {
                response.map(Some)
            }
        },
    }
}

pub(super) async fn read_text_with_cancel(
    response: reqwest::Response,
    cancel: &Arc<AtomicBool>,
) -> Result<Option<String>, reqwest::Error> {
    if cancel.load(Ordering::SeqCst) {
        return Ok(None);
    }
    let text = tokio::select! {
        biased;
        _ = wait_for_cancel(cancel) => return Ok(None),
        text = response.text() => text,
    };
    if cancel.load(Ordering::SeqCst) {
        Ok(None)
    } else {
        text.map(Some)
    }
}

pub(super) fn cancelled_round_outcome() -> RoundOutcome {
    RoundOutcome {
        text: String::new(),
        thinking: String::new(),
        tool_calls: Vec::new(),
        provider_history_items: Vec::new(),
        usage: Default::default(),
        ttft_ms: None,
        stop_reason: Some("cancelled".to_string()),
    }
}
