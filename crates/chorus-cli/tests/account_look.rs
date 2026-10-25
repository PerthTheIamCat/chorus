//! Account look (name / icon / colour) and unique directory names.
//! Own test binary: it sets `HOME`, which the unit tests also do.

use chorus_cli::accounts::{add, free_name, list, meta, set_meta, Meta};

#[test]
fn meta_is_cleaned_and_names_are_unique() {
    let home = tempfile::tempdir().unwrap();
    std::env::set_var("HOME", home.path());

    add("claude", "work").unwrap();
    set_meta(
        "claude",
        Some("work"),
        Meta {
            label: Some("  งานหลัก \n".into()),
            icon: Some("sf:briefcase.fill".into()),
            color: Some("#12aB9f".into()),
        },
    )
    .unwrap();
    assert_eq!(meta("claude", Some("work")).label.as_deref(), Some("งานหลัก"));
    assert_eq!(meta("claude", Some("work")).icon.as_deref(), Some("sf:briefcase.fill"));

    // Junk is dropped rather than stored.
    let bad = Meta { label: Some("   ".into()), icon: Some("../x y".into()), color: Some("red".into()) };
    assert_eq!(bad.cleaned(), Meta::default());

    // The default login keeps its look without becoming an "account".
    set_meta("claude", None, Meta { label: Some("ส่วนตัว".into()), ..Default::default() }).unwrap();
    assert_eq!(meta("claude", None).label.as_deref(), Some("ส่วนตัว"));
    assert_eq!(list("claude"), vec!["work".to_string()]);

    assert_eq!(free_name("claude", Some("work"), None), "work-2");
    assert_eq!(free_name("claude", None, Some("My Team!")), "my-team");
    assert_eq!(free_name("claude", None, Some("ทีม")), "account");

    // Clearing removes the file.
    set_meta("claude", Some("work"), Meta::default()).unwrap();
    assert_eq!(meta("claude", Some("work")), Meta::default());
}
