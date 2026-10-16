//! Daemon server lifecycle, socket listener creation, and connection handling.

use crate::config::{expand_tilde, DaemonConfig};
use crate::error::Result;
use crate::events::EventBroadcaster;
use crate::http::{handle_client, DaemonStream};
use crate::interaction::InteractionManager;
use crate::lifecycle::{install_signal_handlers, LifecycleManager, LifecycleMode};
use crate::pairing::PairingManager;
use rusqlite::Connection;
use std::net::{TcpListener, ToSocketAddrs};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

const MAX_CONNECTIONS: usize = 64;

/// Returned automatically when a request or event stream ends, including on panic.
struct ConnectionPermit(Arc<AtomicUsize>);

impl ConnectionPermit {
    fn acquire(active: &Arc<AtomicUsize>) -> Option<Self> {
        active
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < MAX_CONNECTIONS).then_some(n + 1)
            })
            .ok()
            .map(|_| Self(active.clone()))
    }
}

impl Drop for ConnectionPermit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Running instance of the Chorus daemon engine.
pub struct DaemonServer {
    config: DaemonConfig,
    socket_path: PathBuf,
    actual_http_addr: Option<String>,
    conn: Arc<Mutex<Connection>>,
    interactions: InteractionManager,
    broadcaster: EventBroadcaster,
    lifecycle: LifecycleManager,
    pairing: PairingManager,
    worker_handles: Mutex<Vec<JoinHandle<()>>>,
}

impl DaemonServer {
    /// Start the daemon server with the provided configuration.
    pub fn start(config: DaemonConfig) -> Result<Self> {
        // Plain HTTP must never expose control or bearer tokens directly to a LAN.
        // Remote access requires an authenticated encrypted tunnel terminating on loopback.
        let http_bind = config
            .http_addr
            .as_ref()
            .map(|addr| {
                let addresses: Vec<_> = addr.to_socket_addrs()?.collect();
                if addresses.is_empty() || addresses.iter().any(|a| !a.ip().is_loopback()) {
                    return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput,
                    "Plain HTTP must bind to loopback; use an encrypted tunnel for remote access"));
                }
                Ok(addresses[0])
            })
            .transpose()?;
        config.prepare_environment()?;

        let socket_path = expand_tilde(&config.socket_path);

        // Open SQLite database and apply migrations
        let db_path = expand_tilde(&config.db_path);
        let raw_conn = if db_path.to_string_lossy() == ":memory:" {
            let c = Connection::open_in_memory()?;
            c.pragma_update(None, "synchronous", "NORMAL")?;
            c.pragma_update(None, "foreign_keys", "ON")?;
            chorus_core::schema::migrate(&c)?;
            c
        } else {
            let c = Connection::open(&db_path)?;
            c.pragma_update(None, "journal_mode", "WAL")?;
            c.pragma_update(None, "synchronous", "NORMAL")?;
            c.pragma_update(None, "foreign_keys", "ON")?;
            chorus_core::schema::migrate(&c)?;
            c
        };

        let conn = Arc::new(Mutex::new(raw_conn));

        // Initialize Interaction state machine
        let interactions = InteractionManager::new(conn.clone())?;

        // Determine starting sequence number from SQLite
        let max_seq: i64 = {
            let c = conn.lock().unwrap();
            c.query_row("SELECT COALESCE(MAX(seq), 0) FROM event", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap_or(0)
        };
        let broadcaster = EventBroadcaster::new(max_seq + 1);

        // Lifecycle manager
        let mode = if config.detached {
            LifecycleMode::BackgroundDetached
        } else {
            LifecycleMode::Foreground
        };
        let lifecycle = LifecycleManager::new(mode);
        install_signal_handlers(lifecycle.clone());

        // Bind Unix domain socket
        let unix_listener = UnixListener::bind(&socket_path)?;
        unix_listener.set_nonblocking(true)?;

        // Apply owner-only permissions (0600)
        config.apply_socket_permissions()?;

        // Bind optional TCP HTTP fallback
        let (tcp_listener, actual_http_addr) = if let Some(addr) = http_bind {
            let listener = TcpListener::bind(addr)?;
            listener.set_nonblocking(true)?;
            let bound_addr = listener.local_addr()?.to_string();
            (Some(listener), Some(bound_addr))
        } else {
            (None, None)
        };

        let pairing = PairingManager::new().with_endpoint(
            actual_http_addr
                .as_ref()
                .map(|addr| format!("http://{addr}")),
        );

        let server = Self {
            config,
            socket_path: socket_path.clone(),
            actual_http_addr,
            conn: conn.clone(),
            interactions: interactions.clone(),
            broadcaster: broadcaster.clone(),
            lifecycle: lifecycle.clone(),
            pairing: pairing.clone(),
            worker_handles: Mutex::new(Vec::new()),
        };

        // Spawn Unix socket listener loop
        let active_connections = Arc::new(AtomicUsize::new(0));
        let u_active = active_connections.clone();
        let u_conn = conn.clone();
        let u_inter = interactions.clone();
        let u_broad = broadcaster.clone();
        let u_life = lifecycle.clone();
        let u_pair = pairing.clone();
        let u_handle = std::thread::spawn(move || {
            while !u_life.is_shutting_down() {
                match unix_listener.accept() {
                    Ok((stream, _)) => {
                        let Some(permit) = ConnectionPermit::acquire(&u_active) else {
                            continue;
                        };
                        let c = u_conn.clone();
                        let i = u_inter.clone();
                        let b = u_broad.clone();
                        let l = u_life.clone();
                        let p = u_pair.clone();
                        std::thread::spawn(move || {
                            let _permit = permit;
                            let _ = handle_client(DaemonStream::Unix(stream), c, i, b, l, p);
                        });
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(25));
                    }
                    Err(_) => break,
                }
            }
        });

        // Spawn TCP listener loop if enabled
        let mut handles = vec![u_handle];
        if let Some(tcp_listener) = tcp_listener {
            let t_active = active_connections;
            let t_conn = conn.clone();
            let t_inter = interactions.clone();
            let t_broad = broadcaster.clone();
            let t_life = lifecycle.clone();
            let t_pair = pairing.clone();
            let t_handle = std::thread::spawn(move || {
                while !t_life.is_shutting_down() {
                    match tcp_listener.accept() {
                        Ok((stream, _)) => {
                            let Some(permit) = ConnectionPermit::acquire(&t_active) else {
                                continue;
                            };
                            let c = t_conn.clone();
                            let i = t_inter.clone();
                            let b = t_broad.clone();
                            let l = t_life.clone();
                            let p = t_pair.clone();
                            std::thread::spawn(move || {
                                let _permit = permit;
                                let _ = handle_client(DaemonStream::Tcp(stream), c, i, b, l, p);
                            });
                        }
                        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(25));
                        }
                        Err(_) => break,
                    }
                }
            });
            handles.push(t_handle);
        }

        *server.worker_handles.lock().unwrap() = handles;

        Ok(server)
    }

    /// Block current thread until daemon is commanded to shut down.
    pub fn wait_for_shutdown(&self) {
        let flag = self.lifecycle.shutdown_flag();
        while !flag.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Stop the server, unlink Unix socket, and join worker threads.
    pub fn stop(&self) {
        self.lifecycle.request_shutdown();

        // Join listener threads
        let mut handles = self.worker_handles.lock().unwrap();
        for h in handles.drain(..) {
            let _ = h.join();
        }

        // Clean up socket file
        if self.socket_path.exists() {
            let _ = std::fs::remove_file(&self.socket_path);
        }
    }

    /// Bound Unix domain socket path.
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Configuration handle.
    pub fn config(&self) -> &DaemonConfig {
        &self.config
    }

    /// Bound HTTP address (if enabled).
    pub fn http_addr(&self) -> Option<&str> {
        self.actual_http_addr.as_deref()
    }

    /// Interaction manager handle.
    pub fn interactions(&self) -> &InteractionManager {
        &self.interactions
    }

    /// Event broadcaster handle.
    pub fn broadcaster(&self) -> &EventBroadcaster {
        &self.broadcaster
    }

    /// Lifecycle manager handle.
    pub fn lifecycle(&self) -> &LifecycleManager {
        &self.lifecycle
    }

    /// Shared SQLite database handle.
    pub fn conn(&self) -> Arc<Mutex<Connection>> {
        self.conn.clone()
    }

    /// Pairing manager handle.
    pub fn pairing(&self) -> &PairingManager {
        &self.pairing
    }
}

impl Drop for DaemonServer {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_permits_cap_workers_and_are_reusable_after_disconnect() {
        let active = Arc::new(AtomicUsize::new(0));
        let permits: Vec<_> = (0..MAX_CONNECTIONS)
            .map(|_| ConnectionPermit::acquire(&active).unwrap())
            .collect();
        assert!(ConnectionPermit::acquire(&active).is_none());
        drop(permits);
        assert_eq!(active.load(Ordering::Acquire), 0);
        assert!(ConnectionPermit::acquire(&active).is_some());
    }
}
