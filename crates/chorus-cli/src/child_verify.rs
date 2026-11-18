//! Checking a child's work before the lead sees it (multi-agent v2, F4).
//!
//! When a writing child finishes, Chorus runs the project's own check
//! ([`VerifyLoop`]) and the child's `done_when` in its worktree. A failure goes
//! back to that same child once, with the output, and the result is checked
//! again. What the lead reads afterwards says which of three things happened:
//! it passed, it still failed after the one retry, or it could not be checked
//! (project not trusted, no sandbox) -- never a silent pass.
//!
//! The commands run project code, so they run only with the user's trust for
//! the project and inside the same sandbox profile a writing turn gets
//! (`chorus_core::trust`). The pieces are injectable so the policy is tested
//! without a sandbox or a real agent.

use chorus_core::trust::{Confine, Consent, TrustStore};
use chorus_core::verify_loop::VerifyLoop;
use std::io::Read;
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

/// What one verification run found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    /// False when nothing could be run (untrusted project, no sandbox).
    pub ran: bool,
    pub passed: bool,
    /// What went wrong, written to be sent back to the child.
    pub feedback: String,
    /// What the checks were, for the label.
    pub what: String,
}

impl Verdict {
    fn skipped(why: impl Into<String>) -> Self {
        Self {
            ran: false,
            passed: false,
            feedback: String::new(),
            what: why.into(),
        }
    }
}

/// The last `max` characters of `text`: errors are at the end.
fn tail(text: &str, max: usize) -> String {
    let n = text.chars().count();
    if n <= max {
        text.trim().to_string()
    } else {
        format!("…{}", text.chars().skip(n - max).collect::<String>().trim())
    }
}

fn timeout() -> Duration {
    Duration::from_secs(
        std::env::var("CHORUS_CHILD_VERIFY_TIMEOUT_S")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(600),
    )
}

/// Run `done_when` through `sh -c` in the consent's workspace; `Ok(None)` is a
/// pass, `Ok(Some(output))` a failure.
fn run_done_when(consent: &Consent, done_when: &str) -> Result<Option<String>, String> {
    let argv = ["sh".to_string(), "-c".to_string(), done_when.to_string()];
    let mut cmd = consent.command(&argv).map_err(|e| e.to_string())?;
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("could not run it: {e}"))?;
    let read = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut buf = String::new();
            if let Some(mut p) = pipe {
                let mut bytes = Vec::new();
                let _ = p.read_to_end(&mut bytes);
                buf = String::from_utf8_lossy(&bytes).into_owned();
            }
            buf
        })
    };
    let out = read(child.stdout.take().map(|p| Box::new(p) as _));
    let err = read(child.stderr.take().map(|p| Box::new(p) as _));
    let deadline = Instant::now() + timeout();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(e.to_string()),
        }
    };
    let text = format!(
        "{}{}",
        out.join().unwrap_or_default(),
        err.join().unwrap_or_default()
    );
    match status {
        Some(s) if s.success() => Ok(None),
        Some(s) => Ok(Some(format!(
            "`{done_when}` exited with {}:\n{}",
            s.code().map_or("a signal".to_string(), |c| c.to_string()),
            tail(&text, 2000)
        ))),
        None => Ok(Some(format!(
            "`{done_when}` did not finish within {} s and was stopped:\n{}",
            timeout().as_secs(),
            tail(&text, 1000)
        ))),
    }
}

/// Check `worktree` (a checkout of `workspace`): the project's own verifier,
/// then `done_when` when there is one.
pub fn verify_child(
    trust: &TrustStore,
    confine: &Confine,
    workspace: &Path,
    worktree: &Path,
    done_when: &str,
) -> Verdict {
    let consent = match trust.consent_for_worktree(workspace, worktree, confine) {
        Ok(c) => c,
        Err(e) => return Verdict::skipped(e.to_string()),
    };
    let mut what = Vec::new();
    match VerifyLoop::verify_workspace(&consent) {
        Ok(res) => {
            if res.checker_used != "none" {
                what.push(res.checker_used.clone());
            }
            if !res.passed {
                return Verdict {
                    ran: true,
                    passed: false,
                    feedback: res.to_agent_feedback(),
                    what: what.join(" + "),
                };
            }
        }
        // A checker that is not installed is not the child's fault.
        Err(e) => what.push(format!("ตัวตรวจของโปรเจกต์รันไม่ได้: {e}")),
    }
    let done_when = done_when.trim();
    if !done_when.is_empty() {
        what.push(format!("done_when `{done_when}`"));
        match run_done_when(&consent, done_when) {
            Ok(None) => {}
            Ok(Some(output)) => {
                return Verdict {
                    ran: true,
                    passed: false,
                    feedback: output,
                    what: what.join(" + "),
                }
            }
            Err(e) => return Verdict::skipped(format!("done_when `{done_when}` รันไม่ได้: {e}")),
        }
    }
    Verdict {
        ran: true,
        passed: true,
        feedback: String::new(),
        what: what.join(" + "),
    }
}

/// A child's turn result: its message, diff stat and the files it touched.
pub type Work = (String, String, Vec<String>);

/// The prompt that sends a failed check back to the child.
pub fn send_back_prompt(feedback: &str) -> String {
    format!(
        "Chorus checked your work and it does not pass yet. Fix it, then say briefly what you \
         changed. Do not weaken or skip the check.\n\n{feedback}"
    )
}

/// Run the gate: verify, send a failure back once, verify again. Returns the
/// work to report (the retry's when there was one) with the label that says
/// what happened appended to its message.
pub fn gate(
    first: Work,
    verify: &mut dyn FnMut() -> Verdict,
    rerun: &mut dyn FnMut(&str) -> Result<Work, String>,
) -> Work {
    let mut work = first;
    let mut v = verify();
    let mut retried = false;
    if v.ran && !v.passed {
        retried = true;
        match rerun(&send_back_prompt(&v.feedback)) {
            Ok(second) => {
                work = second;
                v = verify();
            }
            Err(e) => {
                work.0.push_str(&format!(
                    "\n\n✗ ตรวจไม่ผ่าน และส่งกลับไปแก้ไม่สำเร็จ: {e}\n{}",
                    tail(&v.feedback, 600)
                ));
                return work;
            }
        }
    }
    let label = match (v.ran, v.passed, retried) {
        (true, true, false) => format!("✓ ตรวจแล้วผ่าน ({})", v.what),
        (true, true, true) => format!("✓ ตรวจไม่ผ่านครั้งแรก ส่งกลับให้แก้ 1 ครั้ง แล้วผ่าน ({})", v.what),
        (true, false, _) => format!(
            "✗ ยังไม่ผ่านหลังส่งกลับให้แก้ 1 ครั้ง ({}):\n{}",
            v.what,
            tail(&v.feedback, 800)
        ),
        (false, _, _) => format!("○ ไม่ได้ตรวจ: {}", v.what),
    };
    work.0.push_str(&format!("\n\n{label}"));
    work
}

#[cfg(test)]
mod tests {
    use super::*;

    fn work(msg: &str) -> Work {
        (msg.into(), "1 file changed".into(), vec!["a.rs".into()])
    }

    fn passing() -> Verdict {
        Verdict {
            ran: true,
            passed: true,
            feedback: String::new(),
            what: "done_when `t`".into(),
        }
    }

    fn failing(fb: &str) -> Verdict {
        Verdict {
            ran: true,
            passed: false,
            feedback: fb.into(),
            what: "done_when `t`".into(),
        }
    }

    #[test]
    fn a_pass_is_labelled_and_never_sent_back() {
        let mut calls = 0;
        let out = gate(work("done"), &mut || passing(), &mut |_| {
            calls += 1;
            Ok(work("x"))
        });
        assert_eq!(calls, 0);
        assert!(
            out.0.starts_with("done") && out.0.contains("✓ ตรวจแล้วผ่าน"),
            "{}",
            out.0
        );
    }

    #[test]
    fn a_failure_goes_back_to_the_child_once_with_the_output_then_passes() {
        let mut verdicts = vec![failing("assertion left == right"), passing()].into_iter();
        let mut sent = Vec::new();
        let out = gate(
            work("first try"),
            &mut || verdicts.next().unwrap(),
            &mut |p| {
                sent.push(p.to_string());
                Ok(work("fixed it"))
            },
        );
        assert_eq!(sent.len(), 1, "sent back exactly once");
        assert!(
            sent[0].contains("assertion left == right"),
            "output travels with it: {}",
            sent[0]
        );
        assert!(
            out.0.starts_with("fixed it"),
            "the retry's message is reported: {}",
            out.0
        );
        assert!(out.0.contains("แล้วผ่าน"), "{}", out.0);
    }

    #[test]
    fn a_second_failure_is_reported_not_retried_again() {
        let mut sent = 0;
        let out = gate(work("first"), &mut || failing("still red"), &mut |_| {
            sent += 1;
            Ok(work("second"))
        });
        assert_eq!(sent, 1);
        assert!(
            out.0.contains("✗ ยังไม่ผ่าน") && out.0.contains("still red"),
            "{}",
            out.0
        );
    }

    #[test]
    fn work_that_could_not_be_checked_says_so() {
        let out = gate(
            work("done"),
            &mut || Verdict::skipped("project not trusted"),
            &mut |_| panic!("nothing to send back"),
        );
        assert!(
            out.0.contains("○ ไม่ได้ตรวจ: project not trusted"),
            "{}",
            out.0
        );
    }

    #[test]
    fn done_when_runs_in_the_worktree_for_a_trusted_project() {
        let home = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        let wt = tempfile::tempdir().unwrap();
        std::fs::write(wt.path().join("ok.txt"), "x").unwrap();
        let trust = TrustStore::at(home.path().join("trust.json"));
        let confine = |argv: &[String], _: &Path| Ok(argv.to_vec());

        // Not trusted: nothing runs.
        let v = verify_child(&trust, &confine, ws.path(), wt.path(), "true");
        assert!(!v.ran, "{v:?}");

        trust.set_trusted(ws.path(), true).unwrap();
        let pass = verify_child(&trust, &confine, ws.path(), wt.path(), "test -f ok.txt");
        assert!(pass.ran && pass.passed, "{pass:?}");
        let fail = verify_child(
            &trust,
            &confine,
            ws.path(),
            wt.path(),
            "echo nope >&2; exit 3",
        );
        assert!(fail.ran && !fail.passed, "{fail:?}");
        assert!(
            fail.feedback.contains("exited with 3") && fail.feedback.contains("nope"),
            "{}",
            fail.feedback
        );
    }
}
