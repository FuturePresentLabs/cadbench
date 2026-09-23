//! Applies a [`Task`]'s rubric to a [`RunOutcome`] and reports what passed.
//!
//! Deterministic checks run directly against captured artifacts — no model
//! call scores anything here, matching the discipline the backends
//! themselves are held to. [`Check::Subjective`] is the one honest exception:
//! it is surfaced as unresolved rather than silently passed, exactly as
//! pcbbench's "vibe" criterion is human-graded for now.

pub use eval::{CriterionResult, ScoreReport, Verdict};

use crate::runner::RunOutcome;
use crate::task::{Check, Task};

/// Scores `outcome` against `task`'s rubric.
#[must_use]
pub fn score(task: &Task, outcome: &RunOutcome) -> ScoreReport {
    let results = task
        .rubric
        .iter()
        .map(|criterion| CriterionResult {
            id: criterion.id.clone(),
            description: criterion.description.clone(),
            verdict: verdict_for(&criterion.check, outcome),
            detail: detail_for(&criterion.check, outcome),
        })
        .collect();

    ScoreReport {
        task_id: task.id.clone(),
        backend: outcome.backend.clone(),
        results,
    }
}

fn verdict_for(check: &Check, outcome: &RunOutcome) -> Verdict {
    match check {
        Check::StagesPass => {
            if !outcome.stages.is_empty() && outcome.stages.iter().all(|s| s.ok()) {
                Verdict::Pass
            } else {
                Verdict::Fail
            }
        }
        Check::MinDecisionConfidence { threshold } => {
            // No decisions recorded is not a vacuous pass: a run that made no
            // typed decisions at all did not exercise what this criterion
            // exists to check, so it fails rather than trivially clearing it.
            if outcome.decisions.is_empty() {
                Verdict::Fail
            } else if outcome.decisions.iter().all(|d| d.confidence >= *threshold) {
                Verdict::Pass
            } else {
                Verdict::Fail
            }
        }
        Check::Conforms => conforms_verdict(outcome),
        Check::Subjective => Verdict::NeedsHuman,
        strict => judge(strict, outcome).0,
    }
}

/// The verdict and its detail for the checks that measure the result rather
/// than the run, computed together so the two cannot disagree.
fn judge(check: &Check, outcome: &RunOutcome) -> (Verdict, String) {
    let pass = |ok: bool, detail: String| (if ok { Verdict::Pass } else { Verdict::Fail }, detail);
    let within = |got: f64, want: f64, tol: f64| (got - want).abs() <= tol;
    let no_step = || (Verdict::Fail, "no STEP report: the step stage did not run or failed".to_owned());
    match check {
        Check::Decision { key, one_of } => {
            let matches = |k: &str| match key.strip_suffix('*') {
                Some(prefix) => k.starts_with(prefix),
                None => k == key,
            };
            let hits: Vec<_> = outcome.decisions.iter().filter(|d| matches(&d.key)).collect();
            if hits.is_empty() {
                return (Verdict::Fail, format!("no decision matched {key:?}"));
            }
            let wrong: Vec<String> = hits
                .iter()
                .filter(|d| !one_of.contains(&d.chosen))
                .map(|d| format!("{} = {} ({}, {:.2})", d.key, d.chosen, d.kind, d.confidence))
                .collect();
            if wrong.is_empty() {
                let got: Vec<String> = hits.iter().map(|d| format!("{} = {}", d.key, d.chosen)).collect();
                (Verdict::Pass, got.join(", "))
            } else {
                (Verdict::Fail, format!("expected one of {one_of:?}; got {}", wrong.join(", ")))
            }
        }
        Check::VolumeMm3 {
            expected, tolerance, ..
        } => match &outcome.step {
            None => no_step(),
            Some(r) => pass(
                within(r.volume_mm3, *expected, *tolerance),
                format!(
                    "{:.4} mm^3 vs {expected:.4} +/- {tolerance} ({:+.4})",
                    r.volume_mm3,
                    r.volume_mm3 - expected
                ),
            ),
        },
        Check::BoundsMm { expected, tolerance } => match outcome.step.as_ref().map(|r| r.bounds_mm) {
            None => no_step(),
            Some(None) => (Verdict::Fail, "the STEP report has no bounds".to_owned()),
            Some(Some(got)) => pass(
                (0..3).all(|i| within(got[i], expected[i], *tolerance)),
                format!(
                    "{:.4} x {:.4} x {:.4} vs {} x {} x {} +/- {tolerance}",
                    got[0], got[1], got[2], expected[0], expected[1], expected[2]
                ),
            ),
        },
        Check::TrueSurfaces { min_cylinders, .. } => match &outcome.step {
            None => no_step(),
            Some(r) => pass(
                r.writer == "occt-brep" && r.surfaces.cylinder >= *min_cylinders,
                format!(
                    "writer {}, {} cylindrical face(s) vs at least {min_cylinders}",
                    r.writer, r.surfaces.cylinder
                ),
            ),
        },
        Check::CutPlan {
            pierces,
            length_mm,
            tolerance_mm,
            ..
        } => match &outcome.cut {
            None => (Verdict::Fail, "no cut report: the cut stage did not run or refused".to_owned()),
            Some(r) => pass(
                r.pierce_count == *pierces && within(r.cut_length_mm, *length_mm, *tolerance_mm),
                format!(
                    "{} pierce(s) vs {pierces}; {:.4} mm of cut vs {length_mm} +/- {tolerance_mm}",
                    r.pierce_count, r.cut_length_mm
                ),
            ),
        },
        Check::CutRefused { contains, .. } => judge(
            &Check::Refuses {
                stage: crate::runner::STAGE_CUT.to_owned(),
                contains: contains.clone(),
            },
            outcome,
        ),
        Check::Refuses { stage, contains } => match outcome.stages.iter().find(|s| &s.name == stage) {
            None => (Verdict::Fail, format!("stage {stage:?} never ran")),
            Some(run) if run.ok() => (Verdict::Fail, format!("stage {stage:?} succeeded: nothing was refused")),
            Some(run) => pass(
                run.stderr.contains(contains.as_str()),
                format!(
                    "stage {stage:?} exited {:?}; stderr {} {contains:?}: {}",
                    run.exit_code,
                    if run.stderr.contains(contains.as_str()) { "says" } else { "does not say" },
                    run.stderr.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim()
                ),
            ),
        },
        Check::StagesPass | Check::MinDecisionConfidence { .. } | Check::Conforms | Check::Subjective => {
            unreachable!("judged by verdict_for/detail_for directly")
        }
    }
}

/// **Known shortcut, not the real check.** Real conformance is "does the
/// geometry satisfy the ISO 1101 feature control frames and datums the
/// design itself declares" — `transmog-editor-agent::contract::ConformanceReport`
/// already computes exactly that, but it is reachable from no CLI (a ~15-line
/// wrapper subcommand is the fix, tracked as follow-up transmog work, not
/// done here). Scoring a library-only capability from a subprocess-only
/// harness is not possible without that wrapper, so this stands in with the
/// weaker, honest proxy available today: the build ran to completion without
/// the kernel refusing anything. That catches a build that fell over; it
/// cannot catch a build that finished but is out of tolerance. Replace this
/// function's body, not its signature, once the wrapper exists — everything
/// that calls [`score`] is already written against "some verdict for
/// `Conforms`," not against how it is computed.
fn conforms_verdict(outcome: &RunOutcome) -> Verdict {
    match &outcome.build {
        Some(build) if build.complete && build.failed.is_none() => Verdict::Pass,
        _ => Verdict::Fail,
    }
}

fn detail_for(check: &Check, outcome: &RunOutcome) -> String {
    match check {
        Check::StagesPass => {
            // The stage and the last thing it said: a failure that does not
            // say why sends whoever reads the report back to rerun it.
            let failed: Vec<String> = outcome
                .stages
                .iter()
                .filter(|s| !s.ok())
                .map(|s| {
                    let last = s.stderr.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("");
                    format!("{} ({})", s.name, last.trim())
                })
                .collect();
            if outcome.stages.is_empty() {
                "no stages ran".to_owned()
            } else if failed.is_empty() {
                format!("{} stage(s), all exited 0", outcome.stages.len())
            } else {
                format!("failed: {}", failed.join(", "))
            }
        }
        Check::MinDecisionConfidence { threshold } => {
            if outcome.decisions.is_empty() {
                "no decisions recorded".to_owned()
            } else {
                let worst = outcome
                    .decisions
                    .iter()
                    .map(|d| d.confidence)
                    .fold(f64::INFINITY, f64::min);
                format!(
                    "{} decision(s), worst confidence {worst:.2} vs threshold {threshold:.2}",
                    outcome.decisions.len()
                )
            }
        }
        Check::Conforms => match &outcome.build {
            None => "build never wrote a status file".to_owned(),
            Some(b) if b.complete && b.failed.is_none() => format!(
                "build completed ({}/{} steps) — proxy check only, not real GD&T conformance \
                 (see conforms_verdict doc comment)",
                b.done, b.total
            ),
            Some(b) => match &b.failed {
                Some(reason) => format!("build did not complete: {reason}"),
                None => format!("build incomplete ({}/{} steps)", b.done, b.total),
            },
        },
        Check::Subjective => "not automated — needs a human".to_owned(),
        strict => judge(strict, outcome).1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::{BuildStatus, DecisionRecord, StageRun};
    use crate::task::Criterion;
    use std::path::PathBuf;

    fn task_with(rubric: Vec<Criterion>) -> Task {
        Task {
            id: "t".into(),
            family: "f".into(),
            brief: "b".into(),
            input: None,
            rubric,
        }
    }

    fn base_outcome() -> RunOutcome {
        RunOutcome {
            task_id: "t".into(),
            backend: "transmog".into(),
            workdir: PathBuf::from("/tmp/x"),
            stages: vec![],
            design_path: None,
            build: None,
            decisions: vec![],
            step: None,
            cut: None,
        }
    }

    #[test]
    fn stages_pass_fails_on_empty_stages_not_a_vacuous_pass() {
        let task = task_with(vec![Criterion {
            id: "stages".into(),
            description: "d".into(),
            check: Check::StagesPass,
        }]);
        let report = score(&task, &base_outcome());
        assert_eq!(report.results[0].verdict, Verdict::Fail);
    }

    #[test]
    fn stages_pass_fails_if_any_stage_is_nonzero() {
        let task = task_with(vec![Criterion {
            id: "stages".into(),
            description: "d".into(),
            check: Check::StagesPass,
        }]);
        let mut outcome = base_outcome();
        outcome.stages = vec![
            StageRun {
                name: "a".into(),
                command: "a".into(),
                exit_code: Some(0),
                stdout: String::new(),
                stderr: String::new(),
            },
            StageRun {
                name: "b".into(),
                command: "b".into(),
                exit_code: Some(1),
                stdout: String::new(),
                stderr: String::new(),
            },
        ];
        let report = score(&task, &outcome);
        assert_eq!(report.results[0].verdict, Verdict::Fail);
        assert!(report.results[0].detail.contains('b'));
    }

    #[test]
    fn confidence_fails_on_no_decisions_not_a_vacuous_pass() {
        let task = task_with(vec![Criterion {
            id: "conf".into(),
            description: "d".into(),
            check: Check::MinDecisionConfidence { threshold: 0.7 },
        }]);
        let report = score(&task, &base_outcome());
        assert_eq!(report.results[0].verdict, Verdict::Fail);
    }

    #[test]
    fn confidence_fails_if_the_worst_decision_misses_the_bar() {
        let task = task_with(vec![Criterion {
            id: "conf".into(),
            description: "d".into(),
            check: Check::MinDecisionConfidence { threshold: 0.7 },
        }]);
        let mut outcome = base_outcome();
        outcome.decisions = vec![
            DecisionRecord {
                key: "a".into(),
                kind: "choice".into(),
                chosen: "x".into(),
                confidence: 0.9,
            },
            DecisionRecord {
                key: "b".into(),
                kind: "choice".into(),
                chosen: "y".into(),
                confidence: 0.5,
            },
        ];
        let report = score(&task, &outcome);
        assert_eq!(report.results[0].verdict, Verdict::Fail);
    }

    #[test]
    fn confidence_passes_when_every_decision_clears_the_bar() {
        let task = task_with(vec![Criterion {
            id: "conf".into(),
            description: "d".into(),
            check: Check::MinDecisionConfidence { threshold: 0.7 },
        }]);
        let mut outcome = base_outcome();
        outcome.decisions = vec![DecisionRecord {
            key: "a".into(),
            kind: "choice".into(),
            chosen: "x".into(),
            confidence: 0.71,
        }];
        let report = score(&task, &outcome);
        assert_eq!(report.results[0].verdict, Verdict::Pass);
    }

    #[test]
    fn conforms_passes_only_on_a_complete_unfailed_build() {
        let task = task_with(vec![Criterion {
            id: "c".into(),
            description: "d".into(),
            check: Check::Conforms,
        }]);

        let mut complete = base_outcome();
        complete.build = Some(BuildStatus {
            schema: "transmog.build.stream.v1".into(),
            total: 9,
            done: 9,
            complete: true,
            failed: None,
        });
        assert_eq!(score(&task, &complete).results[0].verdict, Verdict::Pass);

        let mut incomplete = base_outcome();
        incomplete.build = Some(BuildStatus {
            schema: "transmog.build.stream.v1".into(),
            total: 9,
            done: 3,
            complete: false,
            failed: Some("kernel refused a boolean".into()),
        });
        assert_eq!(score(&task, &incomplete).results[0].verdict, Verdict::Fail);

        assert_eq!(
            score(&task, &base_outcome()).results[0].verdict,
            Verdict::Fail
        );
    }

    #[test]
    fn subjective_needs_human_never_pass_or_fail() {
        let task = task_with(vec![Criterion {
            id: "vibe".into(),
            description: "d".into(),
            check: Check::Subjective,
        }]);
        let report = score(&task, &base_outcome());
        assert_eq!(report.results[0].verdict, Verdict::NeedsHuman);
        assert!(
            report.all_automated_pass(),
            "needs-human must not fail the run"
        );
        assert_eq!(report.needs_human(), vec!["vibe"]);
    }

    #[test]
    fn a_real_fail_shows_up_in_all_automated_pass() {
        let task = task_with(vec![Criterion {
            id: "stages".into(),
            description: "d".into(),
            check: Check::StagesPass,
        }]);
        let report = score(&task, &base_outcome());
        assert!(!report.all_automated_pass());
    }
}
