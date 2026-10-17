//! The SwiftUI shell has no test target, so the security-relevant parts of
//! it are pinned here as source contracts. Each assertion names the finding
//! it guards (security review 2026-10-17).

use std::path::PathBuf;

fn app(rel: &str) -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../apps/macos").join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

/// The Info.plist heredoc inside build.sh.
fn info_plist() -> String {
    let sh = app("build.sh");
    let start = sh.find("cat > \"$APP/Contents/Info.plist\"").expect("plist heredoc");
    let end = sh[start..].find("\nPLIST").expect("heredoc end") + start;
    sh[start..end].to_string()
}

/// P3-7: TCC terminates an app that touches the mic or speech recognizer
/// without these, and the agreed floor is macOS 14.
#[test]
fn info_plist_declares_usage_keys_and_macos_14() {
    let plist = info_plist();
    for key in ["NSMicrophoneUsageDescription", "NSSpeechRecognitionUsageDescription"] {
        assert!(plist.contains(&format!("<key>{key}</key>")), "missing {key}");
    }
    assert!(
        plist.contains("<key>LSMinimumSystemVersion</key>    <string>14.0</string>"),
        "LSMinimumSystemVersion must be 14.0"
    );
}

/// P3-7: dictation audio must not go to Apple's servers.
#[test]
fn dictation_is_on_device_only() {
    let src = app("Sources/Chorus/VoiceDictationManager.swift");
    assert!(src.contains("requiresOnDeviceRecognition = true"));
    assert!(src.contains("supportsOnDeviceRecognition"));
}

/// P3-7: session goals reach the system Spotlight index only after opt-in.
#[test]
fn spotlight_indexing_is_opt_in() {
    let src = app("Sources/Chorus/SpotlightIndexer.swift");
    assert!(src.contains("guard Self.isEnabled"), "indexSession must check the setting");
    assert!(src.contains("UserDefaults.standard.bool(forKey: settingKey)"), "absent key = off");
    let settings = app("Sources/Chorus/Settings.swift");
    assert!(settings.contains("@AppStorage(SpotlightIndexer.settingKey) private var spotlightIndexing = false"));
}

/// P1: a finished turn runs project code only for a trusted project.
#[test]
fn post_turn_verify_is_gated_on_trust() {
    let src = app("Sources/Chorus/ContentView.swift");
    assert!(src.contains("if !workspace.isEmpty && engine.projectTrusted {"));
    assert!(!src.contains("if !workspace.isEmpty {\n                    engine.verifyWorkspace"));
}

/// P2-2 / P3-3: the pairing sheet trusts Rust for the endpoint and only
/// drops a device row once the daemon confirmed the revoke.
#[test]
fn pairing_view_has_no_hardcoded_port_and_no_optimistic_revoke() {
    let src = app("Sources/Chorus/RemotePairingView.swift");
    assert!(!src.contains(":24678"), "endpoint must come from the daemon's bound port");
    assert!(src.contains("resp.qr_payload"));
    let revoke = &src[src.find("private func revokeDevice").unwrap()..];
    let remove = revoke.find("pairedDevices.removeAll").unwrap();
    let check = revoke.find("resp?.ok == true").unwrap();
    assert!(check < remove, "row removal must be conditional on ok");
}
