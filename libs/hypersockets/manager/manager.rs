use crossbeam_channel::{unbounded, Receiver, Sender};
use crate::core::{ClientEvent, ConnectionState, Metrics, WebSocketClient};
use crate::traits::{HyperSocketError, MessageRouter, Result, WsMessage};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use parking_lot::RwLock;
use tracing::{debug, info, warn};

/// Unique identifier for a client
pub type ClientId = String;

/// Command for managing clients
#[derive(Debug)]
pub enum ManagerCommand {
    /// Add a new client with the given ID
    AddClient {
        id: ClientId,
        send_response: Sender<Result<()>>,
    },
    /// Remove a client by ID
    RemoveClient {
        id: ClientId,
        send_response: Sender<Result<()>>,
    },
    /// Send a message to a specific client
    SendTo {
        id: ClientId,
        message: WsMessage,
        send_response: Sender<Result<()>>,
    },
    /// Send a message to all clients
    Broadcast {
        message: WsMessage,
        send_response: Sender<usize>,
    },
    /// Get metrics for a specific client
    GetMetrics {
        id: ClientId,
        send_response: Sender<Option<Metrics>>,
    },
    /// Get all client IDs
    ListClients {
        send_response: Sender<Vec<ClientId>>,
    },
    /// Get all client statuses
    GetAllStatuses {
        send_response: Sender<HashMap<ClientId, ConnectionState>>,
    },
    /// Shutdown all clients
    Shutdown,
}

/// Manager for multiple WebSocket clients
///
/// Provides centralized control over multiple WebSocket connections,
/// including broadcasting messages, health monitoring, and automatic
/// reconnection management.
///
/// # Type Parameters
/// - `R`: MessageRouter implementation
/// - `M`: Message type (determined by router)
pub struct ClientManager<R, M>
where
    R: MessageRouter<Message = M>,
    M: Send + std::fmt::Debug + 'static,
{
    clients: Arc<RwLock<HashMap<ClientId, WebSocketClient<R, M>>>>,
    #[allow(dead_code)]
    command_tx: Sender<ManagerCommand>,
    #[allow(dead_code)]
    command_rx: Receiver<ManagerCommand>,
    /// Shared shutdown flag for all managed clients
    shutdown_flag: Arc<AtomicBool>,
    /// Shared halted flag - indicates when any client is disconnected (but not shutting down)
    halted_flag: Arc<AtomicBool>,
    /// Set of currently disconnected client IDs
    disconnected_clients: Arc<RwLock<HashSet<ClientId>>>,
    /// State monitor task handle
    monitor_handle: Arc<RwLock<Option<tokio::task::JoinHandle<()>>>>,
}

impl<R, M> ClientManager<R, M>
where
    R: MessageRouter<Message = M>,
    M: Send + std::fmt::Debug + 'static,
{
    /// Create a new client manager with a shutdown flag
    ///
    /// # Arguments
    /// * `shutdown_flag` - Shared atomic flag that controls shutdown of all managed clients
    ///
    /// # Example
    /// ```ignore
    /// use std::sync::Arc;
    /// use std::sync::atomic::AtomicBool;
    ///
    /// let shutdown_flag = Arc::new(AtomicBool::new(true));
    /// let manager = ClientManager::new(shutdown_flag.clone());
    ///
    /// // Build clients with the same flag
    /// let client = hypersockets::builder()
    ///     .url("wss://api.example.com")
    ///     .router(MyRouter, |routing| routing.handler(Route::Main, MyHandler))
    ///     .shutdown_flag(shutdown_flag.clone())
    ///     .build()
    ///     .await?;
    ///
    /// manager.add_client("my-client", client)?;
    /// ```
    pub fn new(shutdown_flag: Arc<AtomicBool>) -> Self {
        let (command_tx, command_rx) = unbounded();
        let halted_flag = Arc::new(AtomicBool::new(false));
        let disconnected_clients = Arc::new(RwLock::new(HashSet::new()));
        let clients = Arc::new(RwLock::new(HashMap::new()));

        let manager = Self {
            clients: Arc::clone(&clients),
            command_tx,
            command_rx,
            shutdown_flag: Arc::clone(&shutdown_flag),
            halted_flag: Arc::clone(&halted_flag),
            disconnected_clients: Arc::clone(&disconnected_clients),
            monitor_handle: Arc::new(RwLock::new(None)),
        };

        // Spawn state monitoring task
        manager.spawn_state_monitor();

        manager
    }

    /// Get a reference to the shutdown flag
    ///
    /// This flag is shared with all clients managed by this manager.
    /// When set to false, all clients will gracefully shut down and
    /// stop reconnecting.
    ///
    /// # Example
    /// ```ignore
    /// // Trigger graceful shutdown of all clients
    /// manager.shutdown_flag().store(false, Ordering::Release);
    /// ```
    pub fn shutdown_flag(&self) -> &Arc<AtomicBool> {
        &self.shutdown_flag
    }

    /// Get a reference to the halted flag
    ///
    /// The halted flag indicates when ANY managed client is disconnected
    /// (but not shutting down). It's automatically managed by the state
    /// monitoring task.
    ///
    /// - `true`: One or more clients are disconnected and reconnecting
    /// - `false`: All clients are connected (or system is shutting down)
    ///
    /// # Example
    /// ```ignore
    /// if manager.halted_flag().load(Ordering::Acquire) {
    ///     println!("⚠️ Some connections down: {:?}", manager.get_disconnected_clients());
    /// }
    /// ```
    pub fn halted_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.halted_flag)
    }

    /// Check if any client is currently halted (disconnected but reconnecting)
    ///
    /// This is a convenience method that reads the halted flag.
    ///
    /// # Returns
    /// - `true`: At least one client is disconnected
    /// - `false`: All clients are connected
    pub fn is_halted(&self) -> bool {
        self.halted_flag.load(Ordering::Acquire)
    }

    /// Get a list of currently disconnected client IDs
    ///
    /// Returns the IDs of all clients that have a halted_flag configured
    /// and are currently disconnected (but not shutting down).
    ///
    /// # Example
    /// ```ignore
    /// let disconnected = manager.get_disconnected_clients();
    /// if !disconnected.is_empty() {
    ///     println!("Disconnected clients: {:?}", disconnected);
    /// }
    /// ```
    pub fn get_disconnected_clients(&self) -> Vec<ClientId> {
        self.disconnected_clients
            .read()
            .iter()
            .cloned()
            .collect()
    }

    /// Get the count of currently disconnected clients
    ///
    /// This is more efficient than calling `get_disconnected_clients().len()`
    /// as it doesn't allocate a vector.
    pub fn disconnected_count(&self) -> usize {
        self.disconnected_clients.read().len()
    }

    /// Add a new client to the manager
    ///
    /// **Important**: Clients should be built with the same shutdown flag
    /// that was passed to the manager's constructor to enable coordinated shutdown.
    ///
    /// # Arguments
    /// * `id` - Unique identifier for the client
    /// * `client` - The WebSocket client to add
    ///
    /// # Example
    /// ```ignore
    /// let shutdown_flag = Arc::new(AtomicBool::new(true));
    /// let manager = ClientManager::new(shutdown_flag.clone());
    ///
    /// let client = hypersockets::builder()
    ///     .url("wss://api.example.com")
    ///     .router(MyRouter, |routing| routing.handler(Route::Main, MyHandler))
    ///     .shutdown_flag(shutdown_flag.clone())  // Use the same flag
    ///     .build()
    ///     .await?;
    ///
    /// manager.add_client("my-client", client)?;
    /// ```
    pub fn add_client(&self, id: impl Into<ClientId>, client: WebSocketClient<R, M>) -> Result<()> {
        let id = id.into();
        let mut clients = self.clients.write();

        if clients.contains_key(&id) {
            return Err(HyperSocketError::Configuration(format!(
                "Client with id '{}' already exists",
                id
            )));
        }

        clients.insert(id.clone(), client);
        info!("Added client '{}'", id);
        Ok(())
    }

    /// Remove a client from the manager
    pub async fn remove_client(&self, id: &str) -> Result<()> {
        let client = {
            let mut clients = self.clients.write();
            clients.remove(id)
        };

        if let Some(client) = client {
            debug!("Removing client '{}'", id);
            client.shutdown().await?;
            info!("Removed client '{}'", id);
            Ok(())
        } else {
            Err(HyperSocketError::Configuration(format!(
                "Client '{}' not found",
                id
            )))
        }
    }

    /// Send a message to a specific client
    pub fn send_to(&self, id: &str, message: WsMessage) -> Result<()> {
        let clients = self.clients.read();
        let client = clients.get(id).ok_or_else(|| {
            HyperSocketError::Configuration(format!("Client '{}' not found", id))
        })?;

        client.send(message)
    }

    /// Broadcast a message to all connected clients
    ///
    /// Returns the number of clients that successfully received the message
    pub fn broadcast(&self, message: WsMessage) -> usize {
        let clients = self.clients.read();
        let mut count = 0;

        for (id, client) in clients.iter() {
            if client.is_connected() {
                match client.send(message.clone()) {
                    Ok(_) => count += 1,
                    Err(e) => warn!("Failed to send to client '{}': {}", id, e),
                }
            }
        }

        count
    }

    /// Get metrics for a specific client
    pub fn get_metrics(&self, id: &str) -> Option<Metrics> {
        let clients = self.clients.read();
        clients.get(id).map(|client| client.metrics())
    }

    /// Get all client IDs
    pub fn list_clients(&self) -> Vec<ClientId> {
        let clients = self.clients.read();
        clients.keys().cloned().collect()
    }

    /// Get connection status for all clients
    pub fn get_all_statuses(&self) -> HashMap<ClientId, ConnectionState> {
        let clients = self.clients.read();
        clients
            .iter()
            .map(|(id, client)| (id.clone(), client.connection_state()))
            .collect()
    }

    /// Get the number of managed clients
    pub fn client_count(&self) -> usize {
        let clients = self.clients.read();
        clients.len()
    }

    /// Get the number of connected clients
    pub fn connected_count(&self) -> usize {
        let clients = self.clients.read();
        clients.values().filter(|c| c.is_connected()).count()
    }

    /// Check if a client exists
    pub fn has_client(&self, id: &str) -> bool {
        let clients = self.clients.read();
        clients.contains_key(id)
    }

    /// Shutdown all clients and the manager
    ///
    /// This sets the shared shutdown flag to false, which triggers
    /// graceful shutdown of all managed clients simultaneously.
    pub async fn shutdown(self) -> Result<()> {
        info!("Shutting down client manager");

        // Set shutdown flag to trigger graceful shutdown of all clients
        self.shutdown_flag.store(false, Ordering::Release);
        info!("Shutdown flag set, clients will stop reconnecting");

        let clients = {
            let mut clients_lock = self.clients.write();
            std::mem::take(&mut *clients_lock)
        };

        for (id, client) in clients {
            debug!("Shutting down client '{}'", id);
            if let Err(e) = client.shutdown().await {
                warn!("Error shutting down client '{}': {}", id, e);
            }
        }

        info!("Client manager shutdown complete");
        Ok(())
    }

    /// Collect all events from all clients (non-blocking)
    ///
    /// Returns a vector of (client_id, event) pairs
    pub fn collect_events(&self) -> Vec<(ClientId, ClientEvent)> {
        let clients = self.clients.read();
        let mut events = Vec::new();

        for (id, client) in clients.iter() {
            while let Some(event) = client.try_recv_event() {
                events.push((id.clone(), event));
            }
        }

        events
    }

    /// Spawn the state monitoring task
    ///
    /// This task continuously monitors connection events from all managed clients
    /// and updates the halted_flag and disconnected_clients set accordingly.
    ///
    /// The monitor only tracks clients that have a halted_flag configured.
    fn spawn_state_monitor(&self) {
        let clients = Arc::clone(&self.clients);
        let shutdown_flag = Arc::clone(&self.shutdown_flag);
        let halted_flag = Arc::clone(&self.halted_flag);
        let disconnected_clients = Arc::clone(&self.disconnected_clients);
        let monitor_handle = Arc::clone(&self.monitor_handle);

        let handle = tokio::spawn(async move {
            debug!("State monitor task started");

            loop {
                // Check shutdown flag first
                if !shutdown_flag.load(Ordering::Acquire) {
                    debug!("Shutdown detected, state monitor exiting");
                    // Clear halted flag on shutdown
                    halted_flag.store(false, Ordering::Release);
                    break;
                }

                // Collect events from all clients
                let events = {
                    let clients_lock = clients.read();
                    let mut events = Vec::new();
                    for (id, client) in clients_lock.iter() {
                        while let Some(event) = client.try_recv_event() {
                            events.push((id.clone(), event, client.halted_flag()));
                        }
                    }
                    events
                };

                // Process events
                for (client_id, event, client_halted_flag) in events {
                    // Only process events for clients with halted_flag configured
                    if client_halted_flag.is_none() {
                        continue;
                    }

                    match event {
                        ClientEvent::Disconnected => {
                            debug!("Client '{}' disconnected", client_id);

                            // Add to disconnected set
                            let mut disc = disconnected_clients.write();
                            disc.insert(client_id.clone());

                            // Update halted flag if shutdown not in progress
                            if shutdown_flag.load(Ordering::Acquire) && !disc.is_empty() {
                                halted_flag.store(true, Ordering::Release);
                                debug!("Halted flag set (disconnected count: {})", disc.len());
                            }
                        }
                        ClientEvent::Connected => {
                            debug!("Client '{}' connected", client_id);

                            // Remove from disconnected set
                            let mut disc = disconnected_clients.write();
                            disc.remove(&client_id);

                            // Clear halted flag if all reconnected
                            if disc.is_empty() {
                                halted_flag.store(false, Ordering::Release);
                                debug!("Halted flag cleared (all clients connected)");
                            }
                        }
                        _ => {
                            // Ignore other events (Reconnecting, Error)
                        }
                    }
                }

                // Sleep briefly to avoid busy-waiting
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }

            debug!("State monitor task stopped");
        });

        // Store the task handle
        *monitor_handle.write() = Some(handle);
    }
}

// Note: No Default implementation since shutdown_flag must be provided
