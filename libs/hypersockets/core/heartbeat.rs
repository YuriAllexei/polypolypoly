//! Heartbeat mechanism for WebSocket connections
//!
//! # Architecture
//!
//! The heartbeat system uses a dedicated Tokio task that runs independently:
//!
//! ```text
//! ┌─────────────────────┐
//! │  Heartbeat Task     │
//! │  (Tokio spawn)      │
//! │                     │
//! │  Every X interval:  │
//! │  1. Wait for tick   │
//! │  2. Send payload ───┼──> Unbounded Channel ──> Main Message Loop ──> WebSocket
//! │  3. Repeat          │
//! └─────────────────────┘
//! ```
//!
//! # Performance
//!
//! - **Non-blocking**: Uses unbounded crossbeam channel, never blocks the main loop
//! - **Independent timing**: Runs in separate task, timing is not affected by message processing
//! - **Zero-copy**: Payload is cloned only when sent, Arc-based sharing where possible
//! - **Graceful shutdown**: Responds to shutdown signals and cleans up properly
//!
//! # Usage
//!
//! Heartbeat is configured via the builder and both interval AND payload are required:
//!
//! ```ignore
//! let client = hypersockets::builder()
//!     .url("wss://api.example.com")
//!     .parser(MyParser)
//!     .state(MyState)
//!     .heartbeat(
//!         Duration::from_secs(30),              // Required: interval
//!         WsMessage::Text("ping".to_string())   // Required: payload
//!     )
//!     .build()
//!     .await?;
//! ```

use crossbeam_channel::{Receiver, Sender};
use crate::traits::WsMessage;
use std::time::Duration;
use tracing::debug;

/// Heartbeat task that sends periodic messages
///
/// This function runs in a dedicated Tokio task and sends the configured
/// heartbeat payload at regular intervals via a crossbeam channel.
///
/// The task will:
/// 1. Wait for the first interval (skips immediate first tick)
/// 2. On each tick, send the payload through the channel
/// 3. Continue until shutdown signal received or channel closed
///
/// # Arguments
/// * `interval` - Duration between heartbeat messages
/// * `payload` - The message to send on each heartbeat
/// * `heartbeat_tx` - Channel to send heartbeat messages to main loop
/// * `shutdown_rx` - Channel to receive shutdown signal
pub async fn heartbeat_task(
    interval: Duration,
    payload: WsMessage,
    heartbeat_tx: Sender<WsMessage>,
    shutdown_rx: Receiver<()>,
) {
    let mut ticker = tokio::time::interval(interval);
    // Skip the first immediate tick - wait for the first interval
    ticker.tick().await;
    // If we miss ticks due to slow processing, skip them rather than bursting
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    debug!("Heartbeat task started with interval: {:?}", interval);

    loop {
        // Check for shutdown signal first (non-blocking)
        match shutdown_rx.try_recv() {
            Ok(_) => {
                debug!("Heartbeat task received shutdown signal");
                break;
            }
            Err(crossbeam_channel::TryRecvError::Disconnected) => {
                debug!("Heartbeat shutdown channel disconnected");
                break;
            }
            Err(crossbeam_channel::TryRecvError::Empty) => {
                // No shutdown signal, continue
            }
        }

        // Wait for next heartbeat tick
        ticker.tick().await;

        // Send heartbeat payload
        debug!("Heartbeat tick - sending payload");
        if heartbeat_tx.send(payload.clone()).is_err() {
            debug!("Heartbeat channel closed, shutting down heartbeat task");
            break;
        }
    }

    debug!("Heartbeat task exiting");
}

/// Spawn a heartbeat task
///
/// Returns channels for receiving heartbeat messages and shutting down the task
pub fn spawn_heartbeat(
    interval: Duration,
    payload: WsMessage,
) -> (tokio::task::JoinHandle<()>, Sender<()>, Receiver<WsMessage>) {
    let (shutdown_tx, shutdown_rx) = crossbeam_channel::bounded(1);
    let (heartbeat_tx, heartbeat_rx) = crossbeam_channel::unbounded();

    let handle = tokio::spawn(async move {
        heartbeat_task(interval, payload, heartbeat_tx, shutdown_rx).await;
    });

    (handle, shutdown_tx, heartbeat_rx)
}
