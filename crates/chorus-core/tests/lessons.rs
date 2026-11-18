//! Lessons drafted from the ledger: each signal, the watermark, the caps,
//! and the "never trust what came from outside" label.

use chorus_core::lessons::{self, Signal, REJECTED_PREFIX};
use chorus_core::memory::{self, ProposalKind};
use chorus_core::store::Store;
use chorus_core::types::{EventKind, NewEvent};
use chorus_core::{actor, ledger};
use rusqlite::Connection;

struct W {
    store: Store,
    sess: i64,
    user: i64,
    agent: i64,
}

impl W {
    fn new() -> Self {
        let store = Store::open_in_memory().unwrap();
        let c = store.conn();
        let user = actor::intern(c, "user", "human").unwrap();
        let agent = actor::intern(c, "codex", "openai").unwrap();
        let sess = ledger::new_session(c, "work", "/tmp/w").unwrap();
        W { store, sess, user, agent }
    }
    fn c(&self) -> &Connection {
        self.store.conn()
    }
    fn add(&self, kind: EventKind, who: i64, body: &str) -> i64 {
        ledger::append(self.c(), &NewEvent::new(self.sess, kind, who, body)).unwrap()
    }
    fn user_says(&self, t: &str) -> i64 {
        self.add(EventKind::Msg, self.user, t)
    }
    fn agent_says(&self, t: &str) -> i64 {
        self.add(EventKind::Msg, self.agent, t)
    }
    fn agent(&self, kind: EventKind, body: &str) -> i64 {
        self.add(kind, self.agent, body)
    }
    fn scan(&self) -> lessons::Scan {
        lessons::scan(self.c(), self.sess).unwrap()
    }
}

fn of(s: &lessons::Scan, sig: Signal) -> Vec<&lessons::LessonDraft> {
    s.drafts.iter().filter(|d| d.signal == sig).collect()
}

#[test]
fn a_command_that_failed_and_then_passed_becomes_a_lesson_with_the_files_between() {
    let w = W::new();
    w.user_says("fix the build");
    w.agent(EventKind::ToolCall, "execute: pnpm test");
    let failed = w.agent(EventKind::ToolResult, "pnpm test → failed");
    w.agent(EventKind::Diff, "--- src/order/repo.ts\n+++ src/order/repo.ts\n- a\n+ b");
    w.agent(EventKind::ToolCall, "execute: pnpm test");
    let fixed = w.agent(EventKind::ToolResult, "pnpm test → completed");
    let s = w.scan();
    let d = of(&s, Signal::FixedAfterFailing);
    assert_eq!(d.len(), 1);
    assert!(d[0].text.contains("`pnpm test`") && d[0].text.contains("src/order/repo.ts"), "{}", d[0].text);
    assert!(d[0].text.chars().count() <= 140);
    assert_eq!(d[0].evidence, format!("session {} · seq {failed} → {fixed}", w.sess));
    assert!(!d[0].suspicious);
}

#[test]
fn a_failure_that_never_got_fixed_and_non_commands_are_not_lessons() {
    let w = W::new();
    w.agent(EventKind::ToolCall, "execute: make");
    w.agent(EventKind::ToolResult, "make → failed");
    w.agent(EventKind::ToolCall, "read: src/a.rs");
    w.agent(EventKind::ToolResult, "src/a.rs → failed");
    w.agent(EventKind::ToolCall, "read: src/a.rs");
    w.agent(EventKind::ToolResult, "src/a.rs → completed");
    assert!(w.scan().drafts.is_empty());
}

#[test]
fn rejected_hunks_a_correction_and_a_dead_handoff_each_leave_a_lesson() {
    let w = W::new();
    w.user_says("add caching");
    w.agent_says("done, added a cache layer");
    let rej = w.add(EventKind::Decision, w.user, &format!("{REJECTED_PREFIX}src/cache.rs, src/lib.rs"));
    let corr = w.user_says("ไม่ใช่ ไม่ต้องใช้ redis ใช้ in-memory พอ");
    w.agent_says("ok");
    w.add(
        EventKind::Handoff,
        w.agent,
        r#"{"goal":"g","done":[],"pending":[],"constraints":[],"files":[],"diff":"","handoff":{"from":"claude","to":"codex","reason":"quota_exhausted","at":0}}"#,
    );
    let hand = ledger::tail(w.c(), w.sess, 1).unwrap()[0].seq;
    w.user_says("hello? anyone there");
    let s = w.scan();

    let r = of(&s, Signal::RejectedHunks);
    assert_eq!(r.len(), 1);
    assert!(r[0].text.contains("src/cache.rs") && r[0].evidence.ends_with(&format!("seq {rej}")));

    let c = of(&s, Signal::Corrected);
    assert_eq!(c.len(), 1, "{:?}", s.drafts);
    assert!(c[0].text.contains("redis") && c[0].evidence.ends_with(&format!("seq {corr}")));

    let h = of(&s, Signal::FailedHandoff);
    assert_eq!(h.len(), 1);
    assert!(h[0].text.contains("claude → codex") && h[0].text.contains("quota exhausted"), "{}", h[0].text);
    assert!(h[0].evidence.ends_with(&format!("seq {hand}")));
}

#[test]
fn a_handoff_the_next_agent_answered_is_not_a_failure_and_a_new_task_is_not_a_correction() {
    let w = W::new();
    w.agent_says("first");
    w.add(
        EventKind::Handoff,
        w.agent,
        r#"{"goal":"g","handoff":{"from":"claude","to":"codex","reason":"rate_limit","at":0}}"#,
    );
    w.agent_says("picked it up");
    w.user_says(&format!("ต่อไปช่วยเขียน test ให้ module ใหม่ {}", "รายละเอียดยาว ".repeat(30)));
    w.user_says("great, now add a README section");
    assert!(w.scan().drafts.is_empty(), "{:?}", w.scan().drafts);
}

#[test]
fn a_rewind_is_a_lesson_about_the_request_the_user_went_back_before() {
    let w = W::new();
    let first = w.user_says("migrate everything to v2");
    w.agent_says("migrated");
    let before = first - 1;
    ledger::fork(w.c(), w.sess, before, "work (ย้อนกลับ)").unwrap();
    let s = w.scan();
    let d = of(&s, Signal::Rewound);
    assert_eq!(d.len(), 1);
    assert!(d[0].text.contains("migrate everything to v2"), "{}", d[0].text);
    assert!(d[0].evidence.ends_with(&format!("seq {first}")));
    // A fork that is not a rewind says nothing.
    let w2 = W::new();
    let m = w2.user_says("x");
    ledger::fork(w2.c(), w2.sess, m, "branch").unwrap();
    assert!(of(&w2.scan(), Signal::Rewound).is_empty());
}

#[test]
fn the_watermark_stops_a_rejected_lesson_from_coming_back() {
    let w = W::new();
    w.agent_says("a");
    w.user_says("อย่าแก้ไฟล์ config");
    let s = w.scan();
    assert_eq!(s.drafts.len(), 1);
    let q = lessons::queue(w.c(), s.drafts.clone()).unwrap();
    assert_eq!(q.queued, 1);
    lessons::mark_scanned(w.c(), w.sess, &s).unwrap();

    let p = memory::pending(w.c()).unwrap();
    assert_eq!(p[0].kind, ProposalKind::Lesson);
    memory::reject(w.c(), p[0].id).unwrap();

    assert!(w.scan().drafts.is_empty(), "same events must not be drafted twice");
    // New events after the watermark are still read.
    w.agent_says("ok");
    w.user_says("ไม่ใช่ อย่าสร้างไฟล์ใหม่");
    assert_eq!(w.scan().drafts.len(), 1);
}

#[test]
fn a_pass_queues_at_most_five_newest_first_and_stops_at_fifteen_waiting() {
    let w = W::new();
    w.agent_says("start");
    let mut last = 0;
    for i in 0..9 {
        last = w.user_says(&format!("ไม่ใช่ แบบที่ {i}"));
        w.agent_says("ok");
    }
    let s = w.scan();
    assert_eq!(s.drafts.len(), 9);
    let q = lessons::queue(w.c(), s.drafts).unwrap();
    assert_eq!((q.queued, q.over_cap), (5, 4));
    let newest = memory::pending(w.c()).unwrap().into_iter().map(|p| p.evidence.unwrap()).collect::<Vec<_>>();
    assert!(newest.iter().any(|e| e.ends_with(&format!("seq {last}"))));

    // Already 5 waiting; fill to the waiting cap with other lessons.
    for i in 0..10 {
        memory::enqueue(
            w.c(),
            &memory::NewProposal { kind: ProposalKind::Lesson, ..memory::NewProposal::fact(1, &format!("filler {i}")) },
        )
        .unwrap();
    }
    w.user_says("อย่า ทำอีก");
    let extra = w.scan().drafts;
    let q = lessons::queue(w.c(), extra).unwrap();
    assert_eq!(q.queued, 0);
}

#[test]
fn imported_or_web_content_marks_every_later_lesson_suspicious() {
    let w = W::new();
    w.add(
        EventKind::Handoff,
        w.agent,
        r#"{"goal":"g","done":[],"pending":[],"constraints":[],"files":[],"diff":"","handoff":{"from":"codex","to":"chorus","reason":"manual","at":0},"source":"codex","turns":3}"#,
    );
    w.agent_says("imported answer");
    w.user_says("อย่าทำแบบนั้น");
    let d = w.scan().drafts;
    assert_eq!(d.len(), 1);
    assert!(d[0].suspicious);
    assert!(lessons::session_tainted(w.c(), w.sess).unwrap());

    let w = W::new();
    w.user_says("read the docs");
    assert!(!lessons::session_tainted(w.c(), w.sess).unwrap());
    w.agent(EventKind::ToolCall, "fetch: https://example.com/x");
    assert!(lessons::session_tainted(w.c(), w.sess).unwrap());
    // The next thing the user says clears the fetch taint for proposals.
    w.user_says("thanks");
    assert!(!lessons::session_tainted(w.c(), w.sess).unwrap());
    // …but lessons drafted after the fetch stay marked.
    w.agent_says("ok");
    w.user_says("อย่า");
    assert!(w.scan().drafts.iter().all(|d| d.suspicious));
}

#[test]
fn queued_lessons_are_masked_deduplicated_and_never_replace_a_fact() {
    let w = W::new();
    memory::add(w.c(), 1, "ผู้ใช้แก้ทาง: \"อย่าแก้ไฟล์ config\"").unwrap();
    w.agent_says("a");
    w.user_says("อย่าแก้ไฟล์ config");
    let s = w.scan();
    let q = lessons::queue(w.c(), s.drafts).unwrap();
    assert_eq!((q.queued, q.known), (0, 1));

    let w = W::new();
    w.agent_says("a");
    let key = format!("sk-ant-api03-{}", "x1y2z3w4".repeat(6));
    w.user_says(&format!("ไม่ใช่ ใช้ key {key}"));
    lessons::queue(w.c(), w.scan().drafts).unwrap();
    let p = memory::pending(w.c()).unwrap();
    assert_eq!(p.len(), 1);
    assert!(!p[0].text.contains(&key), "{}", p[0].text);
    assert_eq!(p[0].replaces, None);
}

#[test]
fn a_patch_names_each_file_once_and_skips_deletions_targets() {
    let patch = "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1 +1 @@\n-x\n+y\n\
                 diff --git a/gone.rs b/gone.rs\n--- a/gone.rs\n+++ /dev/null\n@@ -1 +0,0 @@\n-z\n\
                 diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -9 +9 @@\n-p\n+q\n";
    assert_eq!(lessons::patch_files(patch), vec!["src/a.rs".to_string()]);
}
