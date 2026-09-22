//! Task definition: a design brief plus a rubric to score the result
//! against. Deliberately CAD-kernel-agnostic — nothing here knows about
//! transmog specifically; that's the runner's job ([`crate::runner`]).

use serde::{Deserialize, Serialize};

/// One eval task: what to design, and what "good" means.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    /// Task id, e.g. `"mounting-plate-v1"`.
    pub id: String,
    /// Part family the backend's design-generation step should target
    /// (a backend-specific string, e.g. `"mounting-plate"`).
    pub family: String,
    /// Free-text design brief, carried through to the backend's decision
    /// layer as context. Never parsed for control flow here or in the
    /// backend — the typed decisions, not the prose, choose the geometry.
    pub brief: String,
    pub rubric: Vec<Criterion>,
}

/// One rubric line item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Criterion {
    pub id: String,
    pub description: String,
    #[serde(flatten)]
    pub check: Check,
}

/// What a criterion actually checks. A real enum (not a bool flag) so a
/// check carries the data it needs — e.g. a confidence threshold — instead
/// of that living somewhere else the two can drift apart.
///
/// `objective`/`subjective` is a property of *which variant* this is, not a
/// separate field: [`Check::Subjective`] is the only kind that isn't
/// automatically scored, so there's nothing to keep in sync.
///
/// Two variants carry over from pcbbench unchanged, because they're about the
/// *agent* rather than the domain: [`Check::StagesPass`] and
/// [`Check::MinDecisionConfidence`]. pcbbench's `drc_clean` has no mechanical
/// analog and is replaced by [`Check::Conforms`], which asks the equivalent
/// question of a machined part: does the geometry actually satisfy the
/// tolerances and datums the design itself declares?
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Check {
    /// Every pipeline stage that actually ran must have exited 0.
    StagesPass,
    /// Every decision in the run's trace must clear this confidence
    /// threshold (0.0-1.0).
    MinDecisionConfidence { threshold: f64 },
    /// Every geometric tolerance the design declares must be met by the
    /// geometry the backend actually produced — ISO 1101 feature control
    /// frames evaluated against real datums, not merely present in the file.
    Conforms,
    /// Not automated — a human fills this in. Named explicitly (not just
    /// "no check implemented yet") so a task file is honest about what it
    /// can't verify itself, and so "later every eval needs to be objective"
    /// has something concrete to point at and shrink over time.
    Subjective,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_round_trips_through_toml() {
        let task = Task {
            id: "mounting-plate-v1".into(),
            family: "mounting-plate".into(),
            brief: "6061-T6 mounting plate, central boss, located bore".into(),
            rubric: vec![
                Criterion {
                    id: "stages".into(),
                    description: "every stage that ran, passed".into(),
                    check: Check::StagesPass,
                },
                Criterion {
                    id: "confidence".into(),
                    description: "decisions are confident".into(),
                    check: Check::MinDecisionConfidence { threshold: 0.7 },
                },
                Criterion {
                    id: "conforms".into(),
                    description: "geometry meets its own GD&T".into(),
                    check: Check::Conforms,
                },
                Criterion {
                    id: "vibe".into(),
                    description: "reads as a designed part, not generated".into(),
                    check: Check::Subjective,
                },
            ],
        };
        let text = toml::to_string_pretty(&task).unwrap();
        let back: Task = toml::from_str(&text).unwrap();
        assert_eq!(back.rubric.len(), 4);
        assert!(
            matches!(back.rubric[1].check, Check::MinDecisionConfidence { threshold } if threshold == 0.7)
        );
        assert!(matches!(back.rubric[2].check, Check::Conforms));
        assert!(matches!(back.rubric[3].check, Check::Subjective));
    }

    /// The shipped task file is part of the harness's contract, not sample
    /// data: if it stops parsing, or quietly loses a criterion, every score
    /// it ever produces is wrong. Parse the real file, not a copy.
    #[test]
    fn the_shipped_mounting_plate_task_parses() {
        let text = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tasks/mounting-plate-v1.toml"
        ))
        .expect("tasks/mounting-plate-v1.toml is readable");
        let task: Task = toml::from_str(&text).expect("it parses");

        assert_eq!(task.id, "mounting-plate-v1");
        assert_eq!(task.family, "mounting-plate");
        assert!(!task.brief.trim().is_empty());

        // The rubric must actually contain the three objective checks the
        // task is supposed to exercise, and the confidence bar must be the
        // 0.7 that pcbbench and cadbench independently converged on.
        assert!(task
            .rubric
            .iter()
            .any(|c| matches!(c.check, Check::StagesPass)));
        assert!(task
            .rubric
            .iter()
            .any(|c| matches!(c.check, Check::Conforms)));
        assert!(task.rubric.iter().any(
            |c| matches!(c.check, Check::MinDecisionConfidence { threshold } if threshold == 0.7)
        ));
        // Rubric ids are how results are keyed; duplicates would silently
        // overwrite each other in any downstream report.
        let mut ids: Vec<&str> = task.rubric.iter().map(|c| c.id.as_str()).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), before, "rubric ids must be unique");
    }

    /// An unrecognised `kind` is a typo or a task written against a newer
    /// harness. Either way, silently skipping it would score the run against
    /// fewer criteria than the task asked for.
    #[test]
    fn an_unknown_check_kind_is_refused() {
        let text = r#"
id = "x"
family = "y"
brief = "z"

[[rubric]]
id = "mystery"
description = "not a kind we implement"
kind = "vibes_check"
"#;
        let err = toml::from_str::<Task>(text).expect_err("unknown kind refused");
        assert!(err.to_string().contains("vibes_check"), "{err}");
    }
}
