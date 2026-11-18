//! Shared helpers for chorus-cli integration tests.

use std::path::PathBuf;

/// The `acp-probe` binary (it serves the mock agent via `--serve-mock`).
///
/// `acp-probe` belongs to chorus-acp, so `CARGO_BIN_EXE_*` is not available
/// here. Cargo's newer build-dir layout puts test executables under
/// `target/<profile>/build/<pkg>/<hash>/out/` instead of `target/<profile>/deps/`,
/// so "pop `deps`" no longer lands in `target/<profile>`. Walk up from the test
/// executable until a directory holds the binary — built binaries still land in
/// `target/<profile>/` under either layout.
pub fn probe_bin() -> PathBuf {
    let exe = std::env::current_exe().expect("test exe");
    let mut tried = Vec::new();
    for dir in exe.ancestors().skip(1) {
        for name in ["acp-probe", "acp-probe.exe"] {
            let p = dir.join(name);
            if p.is_file() {
                return p;
            }
            tried.push(p);
        }
    }
    panic!(
        "acp-probe not found above {} — build it first (`cargo build --workspace --all-targets`, CLAUDE.md 7.8). Tried: {:?}",
        exe.display(),
        tried
    );
}
