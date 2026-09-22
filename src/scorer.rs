//! Applies a [`Task`]'s rubric to a [`RunOutcome`] and reports what passed.
//!
//! Deterministic checks run directly against captured artifacts — no model
//! call scores anything here, matching the discipline the backends
//! themselves are held to. [`Check::Subjective`] is the one honest exception:
//! it is surfaced as unresolved rather than silently passed, exactly as
//! pcbbench's "vibe" criterion is human-graded for now.

use serde::{Deserialize, Serialize};

use crate::runner::RunOutcome;
use crate::task::{Check, Task};

/// One criterion's outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Verdict {
    Pass,
    Fail,
    /// [`Check::Subjective`]: not automated, needs a human to fill in.
    NeedsHuman,
}

/// One rubric line item, scored.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CriterionResult {
    pub id: String,
    pub description: String,
    pub verdict: Verdict,
    /// Why, in the scorer's own words — never blank on a `Fail`, so a
    /// failing run is diagnosable from the result file alone.
    pub detail: String,
}

/// A full scored run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoreReport {
    pub task_id: String,
    pub backend: String,
    pub results: Vec<CriterionResult>,
}

impl ScoreReport {
    /// Every automated criterion passed. [`Verdict::NeedsHuman`] does not
    /// count against this — an unresolved subjective check is not a failure,
    /// it is exactly what it says: unresolved.
    #[must_use]
    pub fn all_automated_pass(&self) -> bool {
        self.results
            .iter()
            .all(|r| !matches!(r.verdict, Verdict::Fail))
    }

    /// Criteria still needing a human, by id.
    #[must_use]
    pub fn needs_human(&self) -> Vec<&str> {
        self.results
            .iter()
            .filter(|r| matches!(r.verdict, Verdict::NeedsHuman))
            .map(|r| r.id.as_str())
            .collect()
    }
}

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
            let failed: Vec<&str> = outcome
                .stages
                .iter()
                .filter(|s| !s.ok())
                .map(|s| s.name.as_str())
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
