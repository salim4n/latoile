//! `/api/events` — the single SSE channel (D10). Resume is `?after=<seq>`
//! or the standard `Last-Event-ID` header; `seq` is the only cursor
//! (contract §4).
//!
//! Mechanism: poll-tailing. The event log has no notify primitive, so a
//! producer task re-reads `events_since(cursor)` on a short interval and
//! pushes new rows into the stream; heartbeat comments keep proxies and
//! browsers from closing idle connections. Poll-tailing is deliberate V1
//! simplicity — when the log grows a notifier, only this file changes.

use crate::state::AppState;
use axum::extract::{Query, State};
use axum::response::sse::{Event, Sse};
use axum::http::HeaderMap;
use serde::Deserialize;
use std::convert::Infallible;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::sync::mpsc;

/// Poll cadence for new rows.
const POLL: Duration = Duration::from_millis(500);
/// A heartbeat comment this often keeps idle connections alive.
const HEARTBEAT: Duration = Duration::from_secs(15);

#[derive(Deserialize)]
pub struct EventsParams {
    after: Option<u64>,
}

pub async fn stream(
    State(state): State<AppState>,
    Query(params): Query<EventsParams>,
    headers: HeaderMap,
) -> Sse<EventStream> {
    let cursor = params.after.or_else(|| last_event_id(&headers)).unwrap_or(0);
    Sse::new(EventStream::spawn(state, cursor))
}

fn last_event_id(headers: &HeaderMap) -> Option<u64> {
    headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
}

/// The receiving half of the producer's channel, as a `Stream`.
pub struct EventStream {
    rx: mpsc::Receiver<Result<Event, Infallible>>,
}

impl EventStream {
    fn spawn(state: AppState, cursor: u64) -> Self {
        let (tx, rx) = mpsc::channel(64);
        tokio::spawn(tail(state, cursor, tx));
        Self { rx }
    }
}

impl futures_core::Stream for EventStream {
    type Item = Result<Event, Infallible>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.rx.poll_recv(cx)
    }
}

/// Read new rows, forward, sleep; a dead receiver means the client hung up.
async fn tail(state: AppState, mut cursor: u64, tx: mpsc::Sender<Result<Event, Infallible>>) {
    let store = state.store.clone();
    let mut heartbeat = tokio::time::interval(HEARTBEAT);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        match store.events_since(cursor).await {
            Ok(events) => {
                for (seq, event) in events {
                    cursor = seq;
                    let frame = Event::default()
                        .id(seq.to_string())
                        .event(event.kind.as_str())
                        .data(event.payload);
                    if tx.send(Ok(frame)).await.is_err() {
                        return;
                    }
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "event tail read failed");
            }
        }

        tokio::select! {
            _ = tokio::time::sleep(POLL) => {}
            _ = heartbeat.tick() => {
                if tx.send(Ok(Event::default().comment("keep-alive"))).await.is_err() {
                    return;
                }
            }
        }
    }
}
