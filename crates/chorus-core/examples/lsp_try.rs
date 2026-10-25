// Manual check: cargo run -p chorus-core --example lsp_try -- <workspace> <file>...
use chorus_core::lsp_client::{diagnose_files, default_wait};
use chorus_core::trust::TrustStore;
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let ws = std::path::PathBuf::from(&args[0]);
    let home = std::env::temp_dir().join("lsp-try-trust");
    let store = TrustStore::at(home.join("t.json"));
    store.set_trusted(&ws, true).unwrap();
    let identity = |argv: &[String], _: &std::path::Path| -> Result<Vec<String>, String> { Ok(argv.to_vec()) };
    let consent = store.consent(&ws, &identity).unwrap();
    let r = diagnose_files(&consent, &args[1..], default_wait()).unwrap();
    println!("{} passed={} {}ms\n{}", r.checker_used, r.passed, r.duration_ms, r.raw_output);
    for d in r.diagnostics { println!("{}:{}:{} {} {}", d.file, d.line, d.col, d.severity, d.message); }
}
