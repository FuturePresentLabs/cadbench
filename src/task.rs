//! Task definition: a design brief plus a rubric to score the result
//! against. Deliberately CAD-kernel-agnostic — nothing here knows about
//! transmog specifically; that's the runner's job ([`crate::runner`]).
//!
//! The container ([`Task`]/[`Criterion`]) is [`eval::Task`]/[`eval::Criterion`]
//! — shared scaffolding every sibling harness in this ecosystem uses.
//! [`Check`] is the one part that's actually CAD-specific.

use std::path::{Path, PathBuf};

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
    /// Every decision whose key matches `key` applied one of `one_of`. A key
    /// ending in `*` matches every key with that prefix (`holes_*`), so a
    /// task need not know how the backend numbers its questions. No matching
    /// decision at all fails: the question the brief settles was never asked.
    ///
    /// Stricter than [`Check::MinDecisionConfidence`]: a model can be sure and
    /// wrong. When the brief settles an answer, assert the answer.
    Decision { key: String, one_of: Vec<String> },
    /// The built solid's volume, mm^3, within `tolerance`. `derivation` is the
    /// hand arithmetic the number came from, so the expectation can be
    /// audited without running anything -- and so it is never a value read
    /// back off the backend under test.
    VolumeMm3 {
        expected: f64,
        tolerance: f64,
        derivation: String,
    },
    /// The built solid's axis-aligned extent in X, Y and Z, mm, each within
    /// `tolerance`.
    BoundsMm { expected: [f64; 3], tolerance: f64 },
    /// The STEP export carries true surfaces (not a faceted mesh), at least
    /// `min_cylinders` of them cylindrical. `derivation` says where the count
    /// comes from.
    TrueSurfaces { min_cylinders: u32, derivation: String },
    /// The part can be cut as a through profile with a jet `kerf_mm` wide:
    /// exactly `pierces` pierces and a jet path `length_mm` long (within
    /// `tolerance_mm`), derived by hand in `derivation`.
    CutPlan {
        kerf_mm: f64,
        pierces: u32,
        length_mm: f64,
        tolerance_mm: f64,
        derivation: String,
    },
    /// The backend refuses: stage `stage` exits non-zero and says `contains`
    /// in its stderr. For tasks whose honest answer is "this cannot be made";
    /// a backend that substitutes something makeable fails.
    Refuses { stage: String, contains: String },
    /// A through cut with a jet `kerf_mm` wide is refused, saying `contains`:
    /// [`Check::Refuses`] for the cut stage, which needs to know the kerf.
    CutRefused { kerf_mm: f64, contains: String },
}

/// What a shape-driven task hands the backend besides its brief: a drawing of
/// the part's shapes, and the height and material the brief does not leave to
/// the design. Read from the task file's `[input]` table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShapeInput {
    /// The SVG, relative to the task file until [`load`] resolves it.
    pub svg: PathBuf,
    /// How tall the part is, mm.
    pub height_mm: f64,
    /// Its material, as the backend names materials.
    pub material: String,
    /// Units for an SVG that does not state its own size.
    #[serde(default)]
    pub units: Option<String>,
}

impl ShapeInput {
    /// The typed input of `task`, if it has one.
    ///
    /// # Errors
    /// The `[input]` table is not a [`ShapeInput`].
    pub fn of(task: &Task) -> Result<Option<Self>, toml::de::Error> {
        task.input
            .clone()
            .map(|table| Self::deserialize(toml::Value::Table(table)))
            .transpose()
    }
}

/// Why a task file could not be loaded.
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("reading {path}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("parsing {path}")]
    Parse {
        path: String,
        #[source]
        source: toml::de::Error,
    },
    #[error("{path}: input {svg} does not exist")]
    MissingInput { path: String, svg: String },
}

/// Loads a task file, resolving its input files against the file's own
/// directory so the task runs the same from anywhere.
///
/// # Errors
/// [`LoadError`] when the file cannot be read or parsed, its `[input]` is
/// malformed, or an input file it names does not exist.
pub fn load(path: &Path) -> Result<Task, LoadError> {
    let shown = path.display().to_string();
    let text = std::fs::read_to_string(path).map_err(|source| LoadError::Read {
        path: shown.clone(),
        source,
    })?;
    let mut task: Task = toml::from_str(&text).map_err(|source| LoadError::Parse {
        path: shown.clone(),
        source,
    })?;
    let input = ShapeInput::of(&task).map_err(|source| LoadError::Parse {
        path: shown.clone(),
        source,
    })?;
    if let Some(mut input) = input {
        let dir = path.parent().unwrap_or_else(|| Path::new("."));
        input.svg = dir.join(&input.svg);
        if !input.svg.is_file() {
            return Err(LoadError::MissingInput {
                path: shown,
                svg: input.svg.display().to_string(),
            });
        }
        let Ok(toml::Value::Table(table)) = toml::Value::try_from(&input) else {
            unreachable!("a ShapeInput always serializes to a table");
        };
        task.input = Some(table);
    }
    Ok(task)
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
            input: None,
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
            // A refusal task's point is that nothing gets built, so it is
            // exempt from the criteria that presume a build.
            &[
                ("a stages_pass criterion (or a refusal)", |r| {
                    refuses(r) || r.iter().any(|c| matches!(c.check, Check::StagesPass))
                }),
                ("a conforms criterion (or a refusal)", |r| {
                    refuses(r) || r.iter().any(|c| matches!(c.check, Check::Conforms))
                }),
                ("a min_decision_confidence criterion (or a refusal)", |r| {
                    refuses(r)
                        || r.iter()
                            .any(|c| matches!(c.check, Check::MinDecisionConfidence { .. }))
                }),
            ],
        );
    }

    fn refuses(rubric: &[Criterion]) -> bool {
        rubric
            .iter()
            .any(|c| matches!(c.check, Check::Refuses { .. } | Check::CutRefused { .. }))
    }

    /// The stricter authoring rules in GUIDELINES.md, enforced over every
    /// shipped task so none of them is only a convention. Each failure names
    /// the file and the rule.
    #[test]
    fn every_shipped_task_follows_the_authoring_guidelines() {
        let dir = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tasks"));
        let mut families_with_shapes = std::collections::BTreeSet::new();
        let mut families_with_refusals = std::collections::BTreeSet::new();
        let mut problems = Vec::new();
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            // G1: inputs parse and exist.
            let task = match load(&path) {
                Ok(task) => task,
                Err(e) => {
                    problems.push(format!("{name}: G1 {e}"));
                    continue;
                }
            };
            let mut bad = |rule: &str, what: String| problems.push(format!("{name}: {rule} {what}"));
            let checks: Vec<&Check> = task.rubric.iter().map(|c| &c.check).collect();
            let refusal = refuses(&task.rubric);
            if task.input.is_some() {
                families_with_shapes.insert(task.family.clone());
                // G2: a built part is checked against a number, not a feeling.
                let geometric = checks
                    .iter()
                    .any(|c| matches!(c, Check::VolumeMm3 { .. } | Check::BoundsMm { .. }));
                if !geometric && !refusal {
                    bad("G2", "has no volume_mm3 or bounds_mm criterion".into());
                }
            }
            if refusal {
                families_with_refusals.insert(task.family.clone());
            }
            let brief = task.brief.to_lowercase();
            let mut subjective = 0;
            for check in &checks {
                match check {
                    // G3: every expected number says where it came from, and
                    // tolerances are tight enough to catch a wrong answer.
                    Check::VolumeMm3 { expected, tolerance, derivation } => {
                        if derivation.trim().is_empty() {
                            bad("G3", "volume_mm3 has no derivation".into());
                        }
                        if *tolerance > 1e-4 * expected.abs() {
                            bad("G3", format!("volume tolerance {tolerance} is looser than 1e-4 of {expected}"));
                        }
                    }
                    Check::BoundsMm { tolerance, .. } if *tolerance > 0.05 => {
                        bad("G3", format!("bounds tolerance {tolerance} mm is looser than 0.05"));
                    }
                    Check::CutPlan { tolerance_mm, derivation, .. } => {
                        if derivation.trim().is_empty() {
                            bad("G3", "cut_plan has no derivation".into());
                        }
                        if *tolerance_mm > 0.05 {
                            bad("G3", format!("cut tolerance {tolerance_mm} mm is looser than 0.05"));
                        }
                    }
                    Check::TrueSurfaces { derivation, .. } if derivation.trim().is_empty() => {
                        bad("G3", "true_surfaces has no derivation".into());
                    }
                    // G4: the brief describes intent; it never names the
                    // machine key of the answer it is testing for.
                    Check::Decision { key, one_of } => {
                        if key.trim().is_empty() || one_of.is_empty() {
                            bad("G4", "decision with an empty key or no accepted answers".into());
                        }
                        for option in one_of {
                            let machine = option.contains('_') || option.chars().any(|c| c.is_ascii_digit());
                            if machine && brief.contains(&option.to_lowercase()) {
                                bad("G4", format!("brief leaks the answer key {option:?}"));
                            }
                        }
                    }
                    Check::Subjective => subjective += 1,
                    _ => {}
                }
            }
            // G5: at most one human-judged criterion, never the only one.
            if subjective > 1 || (subjective == 1 && checks.len() == 1) {
                bad("G5", format!("{subjective} subjective criteria out of {}", checks.len()));
            }
        }
        // G6: a family that can build things can also be asked for something
        // it must refuse; one that never is cannot tell a refusal from a
        // silent substitution.
        for family in families_with_shapes.difference(&families_with_refusals) {
            problems.push(format!("family {family}: G6 has no refusal task"));
        }
        assert!(problems.is_empty(), "authoring guideline violations:\n  {}", problems.join("\n  "));
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
