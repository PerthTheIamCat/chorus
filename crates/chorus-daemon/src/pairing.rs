//! Remote Mobile Companion & QR-Code Pairing Manager (Phase 6).
//!
//! Provides cryptographically secure pairing handshakes, ephemeral QR tokens,
//! and device token verification for remote iOS/iPadOS companion clients.

use crate::error::{DaemonError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// Ephemeral pairing session waiting for mobile QR scan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairingSession {
    pub daemon_id: String,
    pub machine_name: String,
    pub pairing_token: String,
    pub endpoint_url: String,
    pub tailscale_ip: Option<String>,
    pub created_at: u64,
    pub expires_at: u64,
}

/// A securely paired mobile companion device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairedDevice {
    pub device_id: String,
    pub device_name: String,
    pub auth_token: String,
    pub paired_at: u64,
    pub last_seen_at: u64,
}

impl PairingSession {
    /// The endpoint the phone should dial: the port the daemon actually
    /// bound (from `endpoint_url`), on the Tailscale address when there is
    /// one. Never a hardcoded port — another local user could bind that.
    pub fn advertised_endpoint(&self) -> String {
        let Some(ip) = self.tailscale_ip.as_deref().filter(|ip| !ip.is_empty()) else {
            return self.endpoint_url.clone();
        };
        let hostport = self
            .endpoint_url
            .split_once("://")
            .map_or(self.endpoint_url.as_str(), |(_, rest)| rest);
        let hostport = hostport.split('/').next().unwrap_or(hostport);
        match hostport.rsplit_once(':').map(|(_, port)| port) {
            Some(port) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => {
                format!("http://{ip}:{port}")
            }
            _ => self.endpoint_url.clone(),
        }
    }

    /// The `chorus://pair` URL encoded into the QR code.
    pub fn qr_payload(&self) -> String {
        format!(
            "chorus://pair?token={}&endpoint={}",
            self.pairing_token,
            self.advertised_endpoint()
        )
    }
}

#[derive(Debug, Clone)]
pub struct PairingManager {
    inner: Arc<Mutex<PairingState>>,
    endpoint_url: Option<String>,
}

#[derive(Debug)]
struct PairingState {
    active_session: Option<PairingSession>,
    devices: HashMap<String, PairedDevice>, // device_id -> PairedDevice
    token_to_device: HashMap<String, String>, // auth_token -> device_id
}

impl Default for PairingManager {
    fn default() -> Self {
        Self::new()
    }
}

impl PairingManager {
    pub fn new() -> Self {
        Self {
            endpoint_url: None,
            inner: Arc::new(Mutex::new(PairingState {
                active_session: None,
                devices: HashMap::new(),
                token_to_device: HashMap::new(),
            })),
        }
    }

    pub fn with_endpoint(mut self, endpoint_url: Option<String>) -> Self {
        self.endpoint_url = endpoint_url;
        self
    }

    pub fn initiate_host_session(&self) -> Result<PairingSession> {
        let endpoint = self.endpoint_url.as_deref().ok_or_else(|| {
            DaemonError::InvalidRequest("HTTP pairing transport is disabled".into())
        })?;
        self.initiate_session(endpoint)
    }

    /// Initiate a new 5-minute QR pairing session.
    pub fn initiate_session(&self, endpoint_url: &str) -> Result<PairingSession> {
        let now = now_secs();
        let pairing_token = secure_token()?;
        let daemon_id = format!("chorus-{}", &pairing_token[..8]);

        let machine_name = std::env::var("USER")
            .map(|u| format!("{u}'s Mac"))
            .unwrap_or_else(|_| "macOS Host".into());

        let session = PairingSession {
            daemon_id,
            machine_name,
            pairing_token,
            endpoint_url: endpoint_url.to_string(),
            tailscale_ip: detect_tailscale_ip(),
            created_at: now,
            expires_at: now + 300, // 5 minutes
        };

        let mut lock = self.inner.lock().unwrap();
        lock.active_session = Some(session.clone());
        Ok(session)
    }

    /// Complete mobile pairing with the scanned token.
    pub fn complete_pairing(&self, pairing_token: &str, device_name: &str) -> Result<PairedDevice> {
        let mut lock = self.inner.lock().unwrap();
        let now = now_secs();

        // Validate session
        let session = lock
            .active_session
            .as_ref()
            .ok_or_else(|| DaemonError::InvalidRequest("no active pairing session".into()))?;

        if session.pairing_token != pairing_token {
            return Err(DaemonError::InvalidRequest("invalid pairing token".into()));
        }

        if now >= session.expires_at {
            lock.active_session = None;
            return Err(DaemonError::InvalidRequest(
                "pairing token has expired".into(),
            ));
        }

        // Generate persistent auth bearer token
        let device_id = format!("device_{}", secure_token()?);
        let auth_token = format!("cpt_{}", secure_token()?);

        let device = PairedDevice {
            device_id: device_id.clone(),
            device_name: device_name.to_string(),
            auth_token: auth_token.clone(),
            paired_at: now,
            last_seen_at: now,
        };

        lock.devices.insert(device_id.clone(), device.clone());
        lock.token_to_device.insert(auth_token, device_id);

        // Consume one-time pairing session
        lock.active_session = None;

        Ok(device)
    }

    /// Verify authorization bearer token from mobile request.
    pub fn verify_token(&self, auth_token: &str) -> Option<PairedDevice> {
        let mut lock = self.inner.lock().unwrap();
        let dev_id = lock.token_to_device.get(auth_token)?.clone();
        if let Some(dev) = lock.devices.get_mut(&dev_id) {
            dev.last_seen_at = now_secs();
            return Some(dev.clone());
        }
        None
    }

    /// List all paired companion devices.
    pub fn list_devices(&self) -> Vec<PairedDevice> {
        let lock = self.inner.lock().unwrap();
        lock.devices.values().cloned().collect()
    }

    /// Revoke a paired companion device.
    pub fn revoke_device(&self, device_id: &str) -> bool {
        let mut lock = self.inner.lock().unwrap();
        if let Some(dev) = lock.devices.remove(device_id) {
            lock.token_to_device.remove(&dev.auth_token);
            true
        } else {
            false
        }
    }
}

/// Use the OS entropy source; fail closed if it is unavailable.
fn secure_token() -> std::io::Result<String> {
    let mut bytes = [0u8; 32];
    // SAFETY: bytes is writable for its full length, below getentropy's 256-byte limit.
    if unsafe { libc::getentropy(bytes.as_mut_ptr().cast(), bytes.len()) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn detect_tailscale_ip() -> Option<String> {
    // Probe for Tailscale IPv4 subnet (100.x.y.z)
    if let Ok(output) = std::process::Command::new("tailscale")
        .args(["ip", "-4"])
        .output()
    {
        if output.status.success() {
            let ip = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !ip.is_empty() {
                return Some(ip);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    fn session(endpoint: &str, ts: Option<&str>) -> PairingSession {
        PairingSession {
            daemon_id: "d".into(),
            machine_name: "m".into(),
            pairing_token: "tok".into(),
            endpoint_url: endpoint.into(),
            tailscale_ip: ts.map(str::to_string),
            created_at: 0,
            expires_at: 0,
        }
    }

    #[test]
    fn advertised_endpoint_uses_the_bound_port_not_24678() {
        assert_eq!(
            session("http://0.0.0.0:51234", Some("100.64.0.7")).advertised_endpoint(),
            "http://100.64.0.7:51234"
        );
        assert_eq!(
            session("http://127.0.0.1:4040", None).advertised_endpoint(),
            "http://127.0.0.1:4040"
        );
        assert_eq!(
            session("http://0.0.0.0:51234", Some("100.64.0.7")).qr_payload(),
            "chorus://pair?token=tok&endpoint=http://100.64.0.7:51234"
        );
    }

    use super::*;

    #[test]
    fn pairing_expires_at_its_deadline_and_replacement_invalidates_old_session() {
        let manager = PairingManager::new();
        let old = manager.initiate_session("http://127.0.0.1:24678").unwrap();
        let current = manager.initiate_session("http://127.0.0.1:24678").unwrap();
        assert_eq!(current.pairing_token.len(), 64);
        assert!(manager
            .complete_pairing(&old.pairing_token, "fixture")
            .is_err());
        manager
            .inner
            .lock()
            .unwrap()
            .active_session
            .as_mut()
            .unwrap()
            .expires_at = now_secs();
        assert!(manager
            .complete_pairing(&current.pairing_token, "fixture")
            .is_err());
        assert!(manager.inner.lock().unwrap().active_session.is_none());
        assert!(manager.list_devices().is_empty());
    }

    #[test]
    fn test_pairing_handshake_flow() {
        let manager = PairingManager::new();

        // 1. Host initiates session
        let session = manager.initiate_session("http://127.0.0.1:4040").unwrap();
        assert_eq!(session.endpoint_url, "http://127.0.0.1:4040");
        assert!(!session.pairing_token.is_empty());

        // 2. Reject wrong token
        let bad = manager.complete_pairing("bad_token", "iPhone 16");
        assert!(bad.is_err());

        // 3. Complete pairing successfully
        let device = manager
            .complete_pairing(&session.pairing_token, "Perth's iPhone 16 Pro")
            .expect("pairing must succeed");

        assert_eq!(device.device_name, "Perth's iPhone 16 Pro");
        assert!(device.auth_token.starts_with("cpt_"));

        // 4. Token cannot be reused (one-time use)
        let reuse = manager.complete_pairing(&session.pairing_token, "Another Phone");
        assert!(reuse.is_err());

        // 5. Verify bearer token
        let verified = manager.verify_token(&device.auth_token);
        assert!(verified.is_some());
        assert_eq!(verified.unwrap().device_id, device.device_id);

        // 6. List devices
        let devices = manager.list_devices();
        assert_eq!(devices.len(), 1);

        // 7. Revoke device
        let revoked = manager.revoke_device(&device.device_id);
        assert!(revoked);
        assert!(manager.verify_token(&device.auth_token).is_none());
    }
}
