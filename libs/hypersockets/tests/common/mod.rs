//! Common test utilities for HyperSockets integration tests
//!
//! This module provides shared utilities for testing WebSocket functionality.

use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::Notify;

/// Macro for verbose test output (controlled by TEST_VERBOSE env var)
#[macro_export]
macro_rules! verbose_println {
    ($($arg:tt)*) => {
        if std::env::var("TEST_VERBOSE").is_ok() {
            println!($($arg)*);
        }
    };
}

/// A simple mock WebSocket server for testing
pub struct MockWsServer {
    pub addr: SocketAddr,
    shutdown: Arc<Notify>,
}

impl MockWsServer {
    /// Create and start a new mock WebSocket server
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let shutdown = Arc::new(Notify::new());
        let shutdown_clone = shutdown.clone();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    result = listener.accept() => {
                        match result {
                            Ok((stream, _)) => {
                                let shutdown = shutdown_clone.clone();
                                tokio::spawn(async move {
                                    Self::handle_connection(stream, shutdown).await;
                                });
                            }
                            Err(e) => {
                                eprintln!("Accept error: {}", e);
                                break;
                            }
                        }
                    }
                    _ = shutdown_clone.notified() => {
                        break;
                    }
                }
            }
        });

        Self { addr, shutdown }
    }

    async fn handle_connection(stream: tokio::net::TcpStream, shutdown: Arc<Notify>) {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::accept_async;

        let ws_stream = match accept_async(stream).await {
            Ok(ws) => ws,
            Err(e) => {
                eprintln!("WebSocket handshake failed: {}", e);
                return;
            }
        };

        let (mut write, mut read) = ws_stream.split();

        loop {
            tokio::select! {
                msg = read.next() => {
                    match msg {
                        Some(Ok(msg)) => {
                            if msg.is_text() || msg.is_binary() {
                                // Echo the message back
                                if write.send(msg).await.is_err() {
                                    break;
                                }
                            } else if msg.is_ping() {
                                // Respond to ping with pong
                                let pong = tokio_tungstenite::tungstenite::Message::Pong(msg.into_data());
                                if write.send(pong).await.is_err() {
                                    break;
                                }
                            } else if msg.is_close() {
                                break;
                            }
                        }
                        Some(Err(_)) | None => break,
                    }
                }
                _ = shutdown.notified() => {
                    break;
                }
            }
        }
    }

    /// Get the WebSocket URL for this server
    pub fn ws_url(&self) -> String {
        format!("ws://{}", self.addr)
    }

    /// Shutdown the server
    pub fn shutdown(&self) {
        self.shutdown.notify_waiters();
    }
}

impl Drop for MockWsServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Test fixture for connection states
pub mod fixtures {
    use hypersockets::core::connection_state::{AtomicConnectionState, ConnectionState};

    pub fn disconnected_state() -> AtomicConnectionState {
        AtomicConnectionState::new(ConnectionState::Disconnected)
    }

    pub fn connected_state() -> AtomicConnectionState {
        AtomicConnectionState::new(ConnectionState::Connected)
    }

    pub fn connecting_state() -> AtomicConnectionState {
        AtomicConnectionState::new(ConnectionState::Connecting)
    }
}
