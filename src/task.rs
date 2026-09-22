//! Task definition: a design brief plus a rubric to score the result
//! against. Deliberately CAD-kernel-agnostic — nothing here knows about
//! transmog specifically; that's the runner's job ([`crate::runner`]).
//!
//! The container ([`Task`]/[`Criterion`]) is [`eval::Task`]/[`eval::Criterion`]
//! — shared scaffolding every sibling harness in this ecosystem uses.
//! [`Check`] is the one part that's actually CAD-specific.

use serde::{Deserialize, Serialize};

/// One eval task: what to design, and what "good" means.
pub type Task = eval::Task<Check>;

/// One rubric line item.
pub type Criterion = eval::Criterion<Check>;

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
    use eval::testing::assert_every_task_sound;
    use std::path::Path;

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

    /// Every shipped task file is part of the harness's contract, not sample
    /// data — see [`eval::testing::assert_every_task_sound`]'s docs. Parses
    /// every real file under `tasks/` rather than naming one, so a new task
    /// file is covered the moment it's added.
    #[test]
    fn every_shipped_task_parses_and_has_a_sound_rubric() {
        let tasks_dir = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tasks"));
        assert_every_task_sound::<Check>(
            tasks_dir,
            &[
                ("a stages_pass criterion", |r| {
                    r.iter().any(|c| matches!(c.check, Check::StagesPass))
                }),
                ("a conforms criterion", |r| {
                    r.iter().any(|c| matches!(c.check, Check::Conforms))
                }),
                ("a min_decision_confidence criterion", |r| {
                    r.iter()
                        .any(|c| matches!(c.check, Check::MinDecisionConfidence { .. }))
                }),
            ],
        );
    }

    /// The original task's specific numbers stay pinned on their own: the
    /// 0.7 confidence bar is a value pcbbench and cadbench independently
    /// converged on, worth catching a silent drift on specifically.
    #[test]
    fn the_shipped_mounting_plate_task_uses_the_shared_confidence_bar() {
        let text = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tasks/mounting-plate-v1.toml"
        ))
        .expect("tasks/mounting-plate-v1.toml is readable");
        let task: Task = toml::from_str(&text).expect("it parses");
        assert_eq!(task.id, "mounting-plate-v1");
        assert!(task.rubric.iter().any(
            |c| matches!(c.check, Check::MinDecisionConfidence { threshold } if threshold == 0.7)
        ));
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
