//! What a listening process *is*, and how to show its command line safely.
//!
//! Naming order (research 02 §4): executable name, then argv for generic
//! runtimes, then `/etc/services` as a "usually X" hint. No port probing.

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NameSource {
    /// Matched the executable name — the strongest signal we have.
    Exe,
    /// Matched the command line of a generic runtime (node, python, …).
    Cmdline,
    /// Only the port is known; `/etc/services` says what usually lives there.
    Port,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ServiceName {
    pub name: String,
    pub source: NameSource,
}

/// Executable file name → service. Matched case-insensitively against the
/// path's last component and the process name (`next-server (v15)` etc).
const BY_EXE: &[(&str, &str)] = &[
    ("redis-server", "Redis"),
    ("valkey-server", "Valkey"),
    ("dragonfly", "Dragonfly"),
    ("keydb-server", "KeyDB"),
    ("postgres", "PostgreSQL"),
    ("postmaster", "PostgreSQL"),
    ("mysqld", "MySQL"),
    ("mariadbd", "MariaDB"),
    ("mongod", "MongoDB"),
    ("mongos", "MongoDB"),
    ("memcached", "Memcached"),
    ("ollama", "Ollama"),
    ("cloudflared", "cloudflared"),
    ("nginx", "nginx"),
    ("httpd", "Apache httpd"),
    ("caddy", "Caddy"),
    ("minio", "MinIO"),
    ("etcd", "etcd"),
    ("consul", "Consul"),
    ("vault", "Vault"),
    ("clickhouse", "ClickHouse"),
    ("clickhouse-server", "ClickHouse"),
    ("influxd", "InfluxDB"),
    ("meilisearch", "Meilisearch"),
    ("typesense-server", "Typesense"),
    ("qdrant", "Qdrant"),
    ("rabbitmq-server", "RabbitMQ"),
    ("nats-server", "NATS"),
    ("temporal", "Temporal"),
    ("mailpit", "Mailpit"),
    ("mailhog", "MailHog"),
    ("chorusd", "Chorus daemon"),
    ("com.docker.backend", "Docker"),
    ("docker-proxy", "Docker"),
    ("next-server", "Next.js"),
    ("llama-server", "llama.cpp"),
    ("lm studio", "LM Studio"),
];

/// argv markers for generic runtimes. Checked in order; the first hit wins.
/// Only consulted when the executable itself is a runtime (see `RUNTIMES`).
const BY_ARGV: &[(&str, &str)] = &[
    ("next dev", "Next.js dev"),
    ("next start", "Next.js"),
    ("next-server", "Next.js"),
    ("vite", "Vite"),
    ("astro", "Astro"),
    ("nuxt", "Nuxt"),
    ("svelte-kit", "SvelteKit"),
    ("remix", "Remix"),
    ("react-scripts", "Create React App"),
    ("webpack", "webpack dev server"),
    ("storybook", "Storybook"),
    ("expo", "Expo"),
    ("wrangler", "Wrangler"),
    ("firebase", "Firebase emulator"),
    ("supabase", "Supabase"),
    ("prisma studio", "Prisma Studio"),
    ("http.server", "Python http.server"),
    ("manage.py runserver", "Django"),
    ("flask", "Flask"),
    ("uvicorn", "Uvicorn"),
    ("gunicorn", "Gunicorn"),
    ("hypercorn", "Hypercorn"),
    ("streamlit", "Streamlit"),
    ("gradio", "Gradio"),
    ("jupyter", "Jupyter"),
    ("mkdocs", "MkDocs"),
    ("rails", "Rails"),
    ("puma", "Puma"),
    ("artisan serve", "Laravel"),
    ("-s localhost", "PHP built-in server"),
    ("elasticsearch", "Elasticsearch"),
    ("kafka", "Kafka"),
    ("zookeeper", "ZooKeeper"),
    ("mcp", "MCP server"),
    ("acp", "Agent bridge"),
];

const RUNTIMES: &[&str] = &[
    "node", "bun", "deno", "python", "python3", "ruby", "php", "java", "npx", "pnpm", "yarn",
    "npm", "uv", "beam.smp", "dotnet",
];

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn is_runtime(exe: &str) -> bool {
    let e = exe.to_ascii_lowercase();
    // `python3.12`, `Python` (framework build), `node22`.
    RUNTIMES.iter().any(|r| {
        e == *r
            || (e.starts_with(r) && e[r.len()..].chars().all(|c| c.is_ascii_digit() || c == '.'))
    })
}

/// Name a process. `argv` is consulted only for runtimes; it never leaves
/// this function except through the matched label.
pub fn name_service(
    path: &str,
    comm: &str,
    argv: &[String],
    port: Option<u16>,
    udp: bool,
) -> Option<ServiceName> {
    let exe = file_name(path);
    let candidates = [exe.to_ascii_lowercase(), comm.to_ascii_lowercase()];
    for (key, name) in BY_EXE {
        if candidates.iter().any(|c| {
            c == key || c.starts_with(&format!("{key} ")) || c.starts_with(&format!("{key}-"))
        }) {
            return Some(ServiceName {
                name: (*name).into(),
                source: NameSource::Exe,
            });
        }
    }
    if is_runtime(exe) || is_runtime(comm) {
        let joined = argv
            .iter()
            .skip(1)
            .map(|a| a.as_str())
            .collect::<Vec<_>>()
            .join(" ")
            .to_ascii_lowercase();
        // `python3 -m http.server` arrives as two args or as `-mhttp.server`.
        for (marker, name) in BY_ARGV {
            if joined.contains(marker) {
                return Some(ServiceName {
                    name: (*name).into(),
                    source: NameSource::Cmdline,
                });
            }
        }
    }
    let port = port?;
    port_hint(port, udp).map(|name| ServiceName {
        name,
        source: NameSource::Port,
    })
}

fn port_hint(port: u16, udp: bool) -> Option<String> {
    // `/etc/services` has no redis entry and calls 5432 "postgresql"; keep a
    // few well-known dev ports that it lacks or names unhelpfully.
    let own = match port {
        6379 => Some("Redis"),
        5432 => Some("PostgreSQL"),
        3306 => Some("MySQL"),
        27017 => Some("MongoDB"),
        11434 => Some("Ollama"),
        9200 => Some("Elasticsearch"),
        _ => None,
    };
    if let Some(n) = own {
        return Some(n.into());
    }
    #[cfg(target_os = "macos")]
    {
        super::darwin::service_for_port(port, udp)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = udp;
        None
    }
}

// ---- masking -------------------------------------------------------------

const SECRET_FLAG_WORDS: &[&str] = &[
    "token",
    "secret",
    "password",
    "passwd",
    "pwd",
    "apikey",
    "api-key",
    "api_key",
    "auth",
    "credential",
    "private-key",
    "private_key",
    "access-key",
    "access_key",
    "session-key",
    "cookie",
    "bearer",
];

const MASK: &str = "••••";

fn flag_is_secret(flag: &str) -> bool {
    let f = flag.trim_start_matches('-').to_ascii_lowercase();
    !f.is_empty() && SECRET_FLAG_WORDS.iter().any(|w| f.contains(w))
}

/// A bare value that looks like a credential: a known prefix, a JWT, or a
/// long run of base64/hex with no path or word structure.
fn value_is_secret(v: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "sk-",
        "sk_",
        "pk_live",
        "rk_live",
        "ghp_",
        "gho_",
        "ghs_",
        "ghu_",
        "github_pat_",
        "glpat-",
        "xoxb-",
        "xoxp-",
        "xapp-",
        "AKIA",
        "ASIA",
        "AIza",
        "ya29.",
        "eyJ",
        "hf_",
        "npm_",
    ];
    if PREFIXES.iter().any(|p| v.starts_with(p)) && v.len() >= 12 {
        return true;
    }
    let long = v.len() >= 32;
    let tokenish = v
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "+/=_-.".contains(c));
    let has_digit = v.chars().any(|c| c.is_ascii_digit());
    let has_alpha = v.chars().any(|c| c.is_ascii_alphabetic());
    long && tokenish && has_digit && has_alpha && !v.contains('/')
}

/// `scheme://user:pass@host` → `scheme://user:••••@host`.
fn mask_userinfo(a: &str) -> Option<String> {
    let (scheme, rest) = a.split_once("://")?;
    let (userinfo, host) = rest.split_once('@')?;
    if host.contains('/') && host.find('/') < userinfo.find('/') {
        return None;
    }
    let (user, _) = userinfo.split_once(':')?;
    Some(format!("{scheme}://{user}:{MASK}@{host}"))
}

/// Mask token-like arguments for display. `--token X`, `--token=X`,
/// `X-Api-Key: …`, credential-shaped values, and passwords in URLs.
/// Research 03 §4.4: the command line is shown masked and never sent to a
/// model; the environment is never shown at all.
pub fn mask_args(argv: &[String]) -> Vec<String> {
    let mut out = Vec::with_capacity(argv.len());
    let mut mask_next = false;
    for a in argv {
        if mask_next {
            mask_next = false;
            if !a.starts_with('-') {
                out.push(MASK.to_string());
                continue;
            }
        }
        if let Some((flag, _)) = a.split_once('=') {
            if flag.starts_with('-') && flag_is_secret(flag)
                || flag.chars().all(|c| c.is_ascii_uppercase() || c == '_') && flag_is_secret(flag)
            {
                out.push(format!("{flag}={MASK}"));
                continue;
            }
        }
        if a.starts_with('-') && flag_is_secret(a) && !a.contains('=') {
            out.push(a.clone());
            mask_next = true;
            continue;
        }
        if let Some((h, _)) = a.split_once(':') {
            if flag_is_secret(h) && !a.contains("://") {
                out.push(format!("{h}: {MASK}"));
                continue;
            }
        }
        if let Some(m) = mask_userinfo(a) {
            out.push(m);
            continue;
        }
        if value_is_secret(a) {
            out.push(MASK.to_string());
            continue;
        }
        out.push(a.clone());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn exe_beats_argv_and_port() {
        let n = name_service(
            "/opt/homebrew/bin/redis-server",
            "redis-server",
            &v(&["redis-server", "*:6379"]),
            Some(6379),
            false,
        )
        .unwrap();
        assert_eq!(
            n,
            ServiceName {
                name: "Redis".into(),
                source: NameSource::Exe
            }
        );
        let n = name_service(
            "/opt/homebrew/opt/postgresql@16/bin/postgres",
            "postgres",
            &[],
            Some(5432),
            false,
        )
        .unwrap();
        assert_eq!(n.name, "PostgreSQL");
    }

    #[test]
    fn runtimes_are_named_from_argv() {
        let py = "/Library/Developer/CommandLineTools/Library/Frameworks/Python3.framework/Versions/3.9/Resources/Python.app/Contents/MacOS/Python";
        let n = name_service(
            py,
            "Python",
            &v(&["python3", "-m", "http.server", "8765"]),
            Some(8765),
            false,
        )
        .unwrap();
        assert_eq!(
            n,
            ServiceName {
                name: "Python http.server".into(),
                source: NameSource::Cmdline
            }
        );
        let n = name_service(
            "/opt/homebrew/bin/node",
            "node",
            &v(&["node", "/x/node_modules/.bin/next", "dev"]),
            Some(3000),
            false,
        )
        .unwrap();
        assert_eq!(n.name, "Next.js dev");
    }

    #[test]
    fn argv_is_ignored_for_non_runtimes() {
        // A random binary whose argv mentions "vite" is not Vite.
        let n = name_service(
            "/usr/local/bin/mytool",
            "mytool",
            &v(&["mytool", "vite"]),
            Some(41234),
            false,
        );
        assert!(n.map(|n| n.source != NameSource::Cmdline).unwrap_or(true));
    }

    #[test]
    fn port_only_is_a_hint() {
        let n = name_service("/x/unknown", "unknown", &[], Some(6379), false).unwrap();
        assert_eq!(
            n,
            ServiceName {
                name: "Redis".into(),
                source: NameSource::Port
            }
        );
    }

    #[test]
    fn masks_token_like_arguments() {
        let got = mask_args(&v(&[
            "language_server",
            "--csrf_token",
            "a1b2c3d4-e5f6",
            "--api-key=sk-live-abcdef123456",
            "--port",
            "8080",
            "postgres://me:hunter2@localhost/db",
            "ghp_0123456789abcdefABCDEF0123456789abcd",
            "Authorization: Bearer xyz",
            "/Users/me/project/src/main.rs",
        ]));
        assert_eq!(
            got,
            v(&[
                "language_server",
                "--csrf_token",
                "••••",
                "--api-key=••••",
                "--port",
                "8080",
                "postgres://me:••••@localhost/db",
                "••••",
                "Authorization: ••••",
                "/Users/me/project/src/main.rs",
            ])
        );
        let joined = got.join(" ");
        for secret in ["a1b2c3d4", "sk-live", "hunter2", "ghp_", "Bearer xyz"] {
            assert!(!joined.contains(secret), "{secret} leaked: {joined}");
        }
    }

    #[test]
    fn ordinary_arguments_survive() {
        let a = v(&[
            "node",
            "server.js",
            "--host",
            "127.0.0.1",
            "-p",
            "3000",
            "--watch",
        ]);
        assert_eq!(mask_args(&a), a);
    }
}
