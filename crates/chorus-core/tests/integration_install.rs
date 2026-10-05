//! Install preview and uninstall must describe the same files. One test fn:
//! it points HOME at a temp dir, and env vars are process-global.

use chorus_core::integration_catalog as cat;
use std::path::Path;

#[test]
fn bundled_skill_plan_install_remove_round_trip() {
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

    let removal = cat::plan("chorus-review", "remove", &project).unwrap();
    assert!(removal.items.iter().any(|i| i.kind == "file" && i.detail.as_deref() == Some(&created)));

    cat::perform("remove", "chorus-review", &project).unwrap();
    assert!(!record.owned_files[0].exists());
    let after = cat::load_state().unwrap();
    assert!(after.installed.is_empty() && after.projects.is_empty());
    assert!(Path::new(home.path()).join(".chorus/tools").exists(), "only the version dir is removed");
}

#[test]
fn unknown_action_is_rejected_by_plan() {
    assert!(cat::plan("rtk", "explode", Path::new("/tmp")).is_err());
}
