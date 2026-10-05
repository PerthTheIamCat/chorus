//! Install preview and uninstall must describe the same files. One test fn:
//! it points HOME at a temp dir, and env vars are process-global.

use chorus_core::integration_catalog as cat;
use std::path::Path;
use std::sync::Mutex;

/// HOME is process-global; tests that repoint it must not overlap.
static HOME_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn bundled_skill_plan_install_remove_round_trip() {
    let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().unwrap();
    std::env::set_var("HOME", home.path());
    let project = home.path().join("demo");
    std::fs::create_dir_all(&project).unwrap();

    let plan = cat::plan("chorus-review", "install_enable", &project).unwrap();
    let phases: Vec<_> = plan.items.iter().map(|i| i.phase).collect();
    assert_eq!(phases, ["fetch", "verify", "verify", "enable"]);
    let created = plan.items[0].detail.clone().unwrap();

    cat::perform("install_enable", "chorus-review", &project).unwrap();
    assert!(cat::progress().is_none(), "progress clears after the action");
    let state = cat::load_state().unwrap();
    let record = &state.installed["chorus-review"];
    assert_eq!(record.owned_files.len(), 1);
    assert!(created.ends_with("chorus-review/1.0.0"), "plan named {created}");
    assert!(record.owned_files[0].exists());
    assert_eq!(cat::enabled_for(&project).unwrap().len(), 1);

    let board = cat::dashboard("chorus-review", &project).unwrap();
    assert_eq!(board.projects, ["demo"]);
    assert!(board.disk_bytes.unwrap() > 0 && board.healthy);
    assert!(board.location.ends_with("tools/chorus-review"));

    let removal = cat::plan("chorus-review", "remove", &project).unwrap();
    assert!(removal.items.iter().any(|i| i.kind == "file"
        && i.detail.as_deref().is_some_and(|d| created.starts_with(d))));

    cat::perform("remove", "chorus-review", &project).unwrap();
    assert!(!record.owned_files[0].exists());
    let after = cat::load_state().unwrap();
    assert!(after.installed.is_empty() && after.projects.is_empty());
    assert!(!home.path().join(".chorus/tools/chorus-review").exists(), "the whole tool dir goes, not just one version");
}

#[test]
fn unknown_action_is_rejected_by_plan() {
    assert!(cat::plan("rtk", "explode", Path::new("/tmp")).is_err());
}

/// PATH without any directory that already holds one of the tools under test,
/// so the developer's own copies are not adopted instead of installed.
fn path_without_tools() {
    let cleaned: Vec<_> = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .filter(|d| !["serena", "graft", "headroom"].iter().any(|t| d.join(t).exists()))
        .collect();
    std::env::set_var("PATH", std::env::join_paths(cleaned).unwrap());
}

/// Needs network + npm. Real install of an npm tool into a temp HOME, then a
/// removal that must leave nothing: not the package, not the npm cache.
/// Run: cargo test -p chorus-core --test integration_install -- --ignored
#[test]
#[ignore]
fn npm_install_remove_leaves_nothing() {
    let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().unwrap();
    std::env::set_var("HOME", home.path());
    path_without_tools();
    let project = home.path().join("demo");
    std::fs::create_dir_all(&project).unwrap();
    cat::perform("install_enable", "graft", &project).unwrap();
    assert!(home.path().join(".chorus/tools/graft/0.18.0/node_modules/.bin/graft").exists());
    assert!(!home.path().join(".npm").exists(), "npm cache must stay inside the tool dir");
    cat::perform("remove", "graft", &project).unwrap();
    assert!(!home.path().join(".chorus/tools/graft").exists());
    assert!(!home.path().join(".npm").exists());
    assert!(cat::load_state().unwrap().installed.is_empty());
}

/// Needs network + uv with Python 3.13. Same guarantee for a uv tool.
#[test]
#[ignore]
fn uv_install_remove_leaves_nothing() {
    let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().unwrap();
    std::env::set_var("HOME", home.path());
    path_without_tools();
    let project = home.path().join("demo");
    std::fs::create_dir_all(&project).unwrap();
    cat::perform("install_enable", "serena", &project).unwrap();
    assert!(home.path().join(".chorus/tools/serena").exists());
    cat::perform("remove", "serena", &project).unwrap();
    assert!(!home.path().join(".chorus/tools/serena").exists());
    assert!(!home.path().join(".cache").exists(), "uv cache must stay inside the tool dir");
    assert!(cat::load_state().unwrap().installed.is_empty());
}

#[test]
fn tool_already_on_the_mac_is_detected_and_never_deleted() {
    use std::os::unix::fs::PermissionsExt;
    let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().unwrap();
    std::env::set_var("HOME", home.path());
    std::env::set_var("PATH", "/usr/bin:/bin"); // keep the developer's own serena out of it
    let bin = home.path().join(".local/bin");
    std::fs::create_dir_all(&bin).unwrap();
    let serena = bin.join("serena");
    std::fs::write(&serena, "#!/bin/sh\necho serena 9.9.9\n").unwrap();
    std::fs::set_permissions(&serena, std::fs::Permissions::from_mode(0o755)).unwrap();

    let entry = cat::get("serena").unwrap();
    let record = cat::current_record(entry, &cat::load_state().unwrap()).expect("found on ~/.local/bin");
    assert_eq!(record.method, cat::InstallMethod::External);
    assert_eq!(record.version, "9.9.9");

    let project = home.path().join("demo");
    std::fs::create_dir_all(&project).unwrap();
    cat::perform("enable", "serena", &project).unwrap();
    assert_eq!(cat::enabled_for(&project).unwrap().len(), 1);
    let plan = cat::plan("serena", "remove", &project).unwrap();
    assert!(plan.items.iter().any(|i| i.kind == "keep"));

    cat::perform("remove", "serena", &project).unwrap();
    assert!(serena.exists(), "a tool Chorus did not install must survive removal");
    assert!(cat::enabled_for(&project).unwrap().is_empty());
}

/// Machine-specific: prints what Chorus finds on this Mac. Run with --nocapture.
#[test]
#[ignore]
fn show_what_this_mac_has() {
    let state = cat::load_state().unwrap();
    for entry in cat::catalog() {
        match cat::current_record(entry, &state) {
            Some(r) => println!("{:10} {:?} {} @ {}", entry.id, r.method, r.version, r.runtime_path.display()),
            None => println!("{:10} not installed", entry.id),
        }
    }
}
