//! The Chorus headless engine daemon (`chorusd`).

use chorus_daemon::config::DaemonConfig;
use chorus_daemon::{DaemonClient, DaemonServer};
use std::path::PathBuf;

fn print_help() {
    println!(
        r#"chorusd v{version} - Chorus Headless Core Daemon

USAGE:
    chorusd [OPTIONS]

OPTIONS:
    --socket <PATH>       Unix Domain Socket path (default: ~/.chorus/engine.sock)
    --http <ADDR>         HTTP fallback bind address (default: 127.0.0.1:24678)
    --no-http             Disable HTTP fallback listener
    --db <PATH>           Path to SQLite database (default: ~/.chorus/ledger.db)
    -d, --background      Run in background detached mode (⌘Q resilience)
    --stop                Send graceful shutdown command to active daemon
    --status              Query status of running daemon
    --ping                Ping running daemon
    -h, --help            Print help information
    -v, --version         Print version information
"#,
        version = env!("CARGO_PKG_VERSION")
    );
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    let mut socket_path: Option<PathBuf> = None;
    let mut http_addr: Option<Option<String>> = None;
    let mut db_path: Option<PathBuf> = None;
    let mut background = false;
    let mut command_stop = false;
    let mut command_status = false;
    let mut command_ping = false;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => {
                print_help();
                return Ok(());
            }
            "-v" | "--version" => {
                println!("chorusd v{}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--socket" => {
                i += 1;
                if i < args.len() {
                    socket_path = Some(PathBuf::from(&args[i]));
                }
            }
            "--http" => {
                i += 1;
                if i < args.len() {
                    http_addr = Some(Some(args[i].clone()));
                }
            }
            "--no-http" => {
                http_addr = Some(None);
            }
            "--db" => {
                i += 1;
                if i < args.len() {
                    db_path = Some(PathBuf::from(&args[i]));
                }
            }
            "-d" | "--background" | "--detach" => {
                background = true;
            }
            "--stop" => {
                command_stop = true;
            }
            "--status" => {
                command_status = true;
            }
            "--ping" => {
                command_ping = true;
            }
            arg => {
                eprintln!("Unknown argument: {arg}");
                print_help();
                std::process::exit(1);
            }
        }
        i += 1;
    }

    let mut config = DaemonConfig::new();
    if let Some(sock) = socket_path {
        config = config.with_socket_path(sock);
    }
    if let Some(http) = http_addr {
        config = config.with_http_addr(http);
    }
    if let Some(db) = db_path {
        config = config.with_db_path(db);
    }
    config = config.with_detached(background);

    let client = DaemonClient::from_uds(&config.socket_path);

    // Client commands targeting running daemon
    if command_ping {
        match client.ping() {
            Ok(true) => {
                println!("pong");
                return Ok(());
            }
            _ => {
                eprintln!("Daemon not responding at {}", config.socket_path.display());
                std::process::exit(1);
            }
        }
    }

    if command_status {
        match client.status() {
            Ok(val) => {
                println!("{}", serde_json::to_string_pretty(&val)?);
                return Ok(());
            }
            Err(e) => {
                eprintln!("Failed to fetch status: {e}");
                std::process::exit(1);
            }
        }
    }

    if command_stop {
        match client.shutdown() {
            Ok(()) => {
                println!("Graceful shutdown requested.");
                return Ok(());
            }
            Err(e) => {
                eprintln!("Failed to stop daemon: {e}");
                std::process::exit(1);
            }
        }
    }

    // Otherwise, start the daemon server
    println!(
        "Starting chorusd v{} [mode: {}]",
        env!("CARGO_PKG_VERSION"),
        if config.detached {
            "background"
        } else {
            "foreground"
        }
    );
    println!("  Unix socket: {}", config.socket_path.display());
    if let Some(ref addr) = config.http_addr {
        println!("  HTTP fallback: http://{addr}");
    }
    println!("  Database: {}", config.db_path.display());

    let server = DaemonServer::start(config)?;
    println!("chorusd is ready to accept connections.");

    server.wait_for_shutdown();
    println!("chorusd stopped cleanly.");

    Ok(())
}
