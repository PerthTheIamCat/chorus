//! Local, repeatable end-to-end comparison of Ollama router models.
//! Usage: cargo run -p chorus-cli --bin router_eval -- CASES.json [--decision-threshold P] MODEL...

use anyhow::{Context, Result};
use chorus_core::router::{
    Candidate, CandidateSet, RouteResult, Router, RouterConfig, TaskSummary,
};
use serde::{Deserialize, Serialize};
use std::time::Instant;

#[derive(Deserialize)]
struct Case {
    language: String,
    text: String,
    kind: String,
    level: String,
    expected: String,
}

#[derive(Serialize)]
struct ResultRow {
    model: String,
    decision_threshold: f64,
    cases: usize,
    correct: usize,
    thai_correct: usize,
    thai_cases: usize,
    english_correct: usize,
    english_cases: usize,
    manual: usize,
    p95_ms: u128,
    failures: Vec<String>,
}

fn candidates() -> CandidateSet {
    let mut all = Vec::new();
    for model in ["luna", "default", "astra"] {
        for effort in ["low", "medium", "high", "max"] {
            all.push(Candidate {
                id: format!("{model}:{effort}"),
                agent: "codex".into(),
                model: model.into(),
                effort: effort.into(),
                media_types: vec!["text/*".into()],
                integrations: vec![],
                ready: true,
                tier: None,
            });
        }
    }
    CandidateSet::new(all)
}

fn run_case(router: &Router, case: &Case, candidates: &CandidateSet) -> String {
    let task = TaskSummary {
        text: case.text.clone(),
        attachments: vec![],
        enabled_integrations: vec![],
        required_integrations: vec![],
    };
    match router.route(&task, candidates) {
        RouteResult::Proposal(proposal) => proposal.candidate_id,
        RouteResult::Unavailable(_) => "manual".into(),
    }
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = args
        .first()
        .context("usage: router_eval CASES.json MODEL...")?;
    let (threshold, model_start) =
        if args.get(1).map(String::as_str) == Some("--decision-threshold") {
            let value = args
                .get(2)
                .context("--decision-threshold needs a value")?
                .parse::<f64>()?;
            (value, 3)
        } else {
            (0.35, 1)
        };
    let models = args
        .get(model_start..)
        .filter(|models| !models.is_empty())
        .context("specify at least one Ollama model")?;
    let cases: Vec<Case> = serde_json::from_slice(&std::fs::read(path).context("read cases")?)?;
    let candidates = candidates();
    for model in models {
        let router =
            Router::new(RouterConfig::local(model).with_decision_min_probability(threshold)?)?;
        // Keep cold-load time separate from steady-state routing latency.
        let _ = run_case(&router, &cases[0], &candidates);
        let mut row = ResultRow {
            model: model.clone(),
            decision_threshold: threshold,
            cases: cases.len(),
            correct: 0,
            thai_correct: 0,
            thai_cases: 0,
            english_correct: 0,
            english_cases: 0,
            manual: 0,
            p95_ms: 0,
            failures: vec![],
        };
        let mut durations = Vec::with_capacity(cases.len());
        for (index, case) in cases.iter().enumerate() {
            let started = Instant::now();
            let actual = run_case(&router, case, &candidates);
            durations.push(started.elapsed().as_millis());
            let correct = actual == case.expected;
            row.correct += usize::from(correct);
            row.manual += usize::from(actual == "manual");
            match case.language.as_str() {
                "th" => {
                    row.thai_cases += 1;
                    row.thai_correct += usize::from(correct);
                }
                "en" => {
                    row.english_cases += 1;
                    row.english_correct += usize::from(correct);
                }
                other => anyhow::bail!("unknown language {other}"),
            }
            if !correct {
                row.failures.push(format!(
                    "{} {} / {}: expected {}, got {}",
                    index + 1,
                    case.kind,
                    case.level,
                    case.expected,
                    actual
                ));
            }
        }
        durations.sort_unstable();
        row.p95_ms = durations[((durations.len() * 95).div_ceil(100)).saturating_sub(1)];
        println!("{}", serde_json::to_string_pretty(&row)?);
    }
    Ok(())
}
