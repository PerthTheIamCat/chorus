//! Text embeddings for the fact store (storage spec §6): cosine dedup and
//! vector retrieval.
//!
//! The model is optional and local. `CHORUS_EMBED_MODEL=nomic-embed-text`
//! (any Ollama embedding model) turns it on; with it unset, or Ollama down,
//! everything falls back to the model-free n-gram path in [`crate::memory`] —
//! nothing here may make a turn fail or wait. Like [`crate::compaction::Ollama`]
//! it is one hand-rolled HTTP POST, no HTTP dependency.

use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Environment variable naming the Ollama embedding model. Unset = off.
pub const EMBED_MODEL_ENV: &str = "CHORUS_EMBED_MODEL";

/// Something that turns text into a vector. `None` means "not available right
/// now" — callers fall back, they never fail.
pub trait Embedder {
    /// Stable id of the model; vectors of different ids are never compared.
    fn id(&self) -> String;
    fn embed(&self, text: &str) -> Option<Vec<f32>>;
}

/// Ollama's `/api/embed`. Short timeouts: this runs when a turn is assembled.
pub struct OllamaEmbedder {
    pub model: String,
    pub host: String,
    pub timeout: Duration,
}

/// After a failure the embedder is skipped for this long, so a stopped Ollama
/// costs one connect attempt a minute rather than one per turn.
const COOLDOWN_MS: u64 = 60_000;
static LAST_FAILURE_MS: AtomicU64 = AtomicU64::new(0);

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

impl OllamaEmbedder {
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            host: std::env::var("OLLAMA_HOST")
                .unwrap_or_else(|_| "127.0.0.1:11434".into())
                .replace("http://", ""),
            timeout: Duration::from_millis(600),
        }
    }

    fn request(&self, text: &str) -> Option<Vec<f32>> {
        use std::io::{BufRead, BufReader, Read, Write};
        use std::net::{TcpStream, ToSocketAddrs};

        let body = serde_json::json!({ "model": self.model, "input": text }).to_string();
        let addr = self.host.to_socket_addrs().ok()?.next()?;
        let mut s = TcpStream::connect_timeout(&addr, self.timeout).ok()?;
        s.set_read_timeout(Some(self.timeout)).ok()?;
        s.set_write_timeout(Some(self.timeout)).ok()?;
        write!(
            s,
            "POST /api/embed HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n{}",
            self.host,
            body.len(),
            body
        )
        .ok()?;
        let mut reader = BufReader::new(s);
        let mut status = String::new();
        reader.read_line(&mut status).ok()?;
        if !status.contains(" 200") {
            return None;
        }
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).ok()?;
            if line.trim().is_empty() {
                break;
            }
        }
        let mut raw = String::new();
        reader.read_to_string(&mut raw).ok()?;
        let v: Value = serde_json::from_str(&raw).ok()?;
        let first = v.get("embeddings")?.as_array()?.first()?.as_array()?;
        let vec: Vec<f32> = first.iter().filter_map(|x| x.as_f64().map(|f| f as f32)).collect();
        (vec.len() == first.len() && !vec.is_empty()).then_some(vec)
    }
}

impl Embedder for OllamaEmbedder {
    fn id(&self) -> String {
        format!("ollama:{}", self.model)
    }

    fn embed(&self, text: &str) -> Option<Vec<f32>> {
        let last = LAST_FAILURE_MS.load(Ordering::Relaxed);
        if last != 0 && now_ms().saturating_sub(last) < COOLDOWN_MS {
            return None;
        }
        let out = self.request(text);
        if out.is_none() {
            LAST_FAILURE_MS.store(now_ms(), Ordering::Relaxed);
        } else if last != 0 {
            LAST_FAILURE_MS.store(0, Ordering::Relaxed);
        }
        out
    }
}

/// The embedder the environment asks for, if any.
pub fn from_env() -> Option<Box<dyn Embedder>> {
    let model = std::env::var(EMBED_MODEL_ENV).ok()?;
    let model = model.trim();
    (!model.is_empty()).then(|| Box::new(OllamaEmbedder::new(model)) as Box<dyn Embedder>)
}

/// Vector → bytes for the `fact_embedding` BLOB (little-endian f32).
pub fn encode(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

pub fn decode(bytes: &[u8]) -> Vec<f32> {
    bytes.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    #[test]
    fn vectors_round_trip_through_bytes() {
        let v = vec![0.25f32, -1.5, 3.0e-3, 0.0];
        assert_eq!(decode(&encode(&v)), v);
    }

    /// A one-shot fake Ollama: answers `/api/embed` with `reply`.
    fn fake_ollama(status: &'static str, reply: &'static str) -> String {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap().to_string();
        std::thread::spawn(move || {
            if let Ok((mut s, _)) = l.accept() {
                // Read the whole request (headers and JSON body) before
                // answering, or the close races the client's write.
                let mut got = Vec::new();
                let mut buf = [0u8; 4096];
                while !String::from_utf8_lossy(&got).contains("\"input\"") {
                    match s.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => got.extend_from_slice(&buf[..n]),
                    }
                }
                let _ = write!(
                    s,
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                    reply.len()
                );
            }
        });
        addr
    }

    fn embedder(host: String) -> OllamaEmbedder {
        OllamaEmbedder { model: "test-embed".into(), host, timeout: Duration::from_millis(500) }
    }

    #[test]
    fn ollama_reply_is_parsed() {
        let host = fake_ollama("200 OK", r#"{"model":"test-embed","embeddings":[[0.5,0.25,-1.0]]}"#);
        assert_eq!(embedder(host).request("hello"), Some(vec![0.5, 0.25, -1.0]));
    }

    #[test]
    fn errors_and_garbage_give_none_not_a_failure() {
        let host = fake_ollama("404 Not Found", r#"{"error":"model not found"}"#);
        assert_eq!(embedder(host).request("x"), None);
        let host = fake_ollama("200 OK", r#"{"embeddings":[]}"#);
        assert_eq!(embedder(host).request("x"), None);
        // Nothing listening at all.
        assert_eq!(embedder("127.0.0.1:1".into()).request("x"), None);
    }
}
