//! Drives a CAD-design backend as a subprocess and collects what it produced.
//!
//! The harness never links a backend. It spawns one, captures stdout, stderr
//! and the exit code of every stage, and then goes looking on disk for the
//! artifacts the stages were supposed to write. That is the same arrangement
//! pcbbench uses to drive legion-of-bom's `lob` CLI, and it is deliberate on
//! both sides: the eval is meant to be FOSS and the thing being measured may
//! not be, so the only contract between them is a command line and a file
//! format. It also means "score a different backend" is a different
//! subprocess and not a rewrite — zoo.dev/KittyCAD's text-to-CAD API is the
//! named next one, and it arrives as another [`Backend`] impl.
//!
//! Nothing in this module understands CAD. It understands processes and file
//! paths. The reading of artifacts into rubric answers is [`crate::scorer`]'s
//! job.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::task::Task;

/// A backend that can attempt a task and leave artifacts behind.
///
/// One method, because a backend has exactly one job. Everything that varies
/// between backends — how it is invoked, how many stages it runs, what it
/// writes — is behind [`Backend::run`], and everything the scorer needs is in
/// the [`RunOutcome`] that comes back. A second implementation should not need
/// this trait to change.
pub trait Backend {
    /// Short identifier recorded in results (`transmog`, `zoo`, ...).
    fn name(&self) -> &str;

    /// Attempts `task`, writing all artifacts under `workdir`.
    ///
    /// # Errors
    /// The backend could not be invoked at all, or a capability the task needs
    /// is missing. A backend that ran and did badly is *not* an error — that
    /// is a [`RunOutcome`] with failing stages, which is a score, not a crash.
    fn run(&self, task: &Task, workdir: &Path) -> Result<RunOutcome, RunError>;
}

/// Failures that stop a run before it can be scored.
#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error("creating work directory {path}")]
    Workdir {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("spawning stage {stage:?}: {program}")]
    Spawn {
        stage: String,
        program: String,
        #[source]
        source: std::io::Error,
    },
    #[error("reading artifact {path}")]
    Artifact {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("parsing artifact {path}")]
    ParseArtifact {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    /// The backend cannot do something the task requires.
    ///
    /// Distinct from a stage failing, and loud on purpose (FPL tenet 12): a
    /// harness that quietly skipped the part a backend cannot do would report
    /// a score that flatters it.
    #[error("backend {backend} cannot {capability}: {detail}")]
    CapabilityMissing {
        backend: String,
        capability: String,
        detail: String,
    },
}

/// One subprocess the backend ran, exactly as it ran.
///
/// The command line is kept as a string so a failed eval can be reproduced by
/// hand from the result file — the runbook-parity check (FPL tenet 8) for a
/// harness is "can a person re-run the step that failed?", and that needs the
/// literal command, not a description of it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageRun {
    /// Stage name within the pipeline (`build-stream`, ...).
    pub name: String,
    /// The command line as invoked.
    pub command: String,
    /// `None` if the process was killed by a signal.
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl StageRun {
    /// Whether this stage exited cleanly.
    #[must_use]
    pub fn ok(&self) -> bool {
        self.exit_code == Some(0)
    }
}

/// One recorded typed decision.
///
/// Field-for-field the record legion-of-bom writes with `lob spec --trace`
/// and pcbbench scores, kept identical so the two benchmarks read the same
/// trace format rather than each inventing one (FPL tenet 1). A CAD backend
/// that wants its decisions scored writes this shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionRecord {
    pub key: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub chosen: String,
    pub confidence: f64,
}

/// `status.json` from `transmog build-stream`, schema
/// `transmog.build.stream.v1`.
///
/// Only the fields the rubric needs are named; the rest of the document
/// (`opaque`, `patches`, `running`, `updatedEpochMs`, per-step timings) is
/// ignored rather than mirrored, so this struct does not have to be revised
/// every time the backend adds a field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildStatus {
    pub schema: String,
    pub total: usize,
    pub done: usize,
    /// Every step ran and none failed.
    pub complete: bool,
    /// The kernel error that stopped the build, if one did.
    pub failed: Option<String>,
}

/// Everything one attempt produced.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunOutcome {
    pub task_id: String,
    pub backend: String,
    pub workdir: PathBuf,
    /// In execution order.
    pub stages: Vec<StageRun>,
    /// The `DesignDocument` RON the backend produced or was given.
    pub design_path: Option<PathBuf>,
    /// Parsed `status.json`, when a build stage ran far enough to write one.
    pub build: Option<BuildStatus>,
    /// The decision trace, if the backend emitted one. Empty is meaningful:
    /// see [`crate::scorer`], which fails a confidence criterion rather than
    /// vacuously passing it over zero decisions.
    pub decisions: Vec<DecisionRecord>,
}

impl RunOutcome {
    /// Whether every stage exited cleanly and the build ran to completion.
    #[must_use]
    pub fn all_stages_ok(&self) -> bool {
        !self.stages.is_empty()
            && self.stages.iter().all(StageRun::ok)
            && self.build.as_ref().is_some_and(|b| b.complete)
    }
}

/// Where the `DesignDocument` under test comes from.
///
/// Today transmog has no "brief in, design out" entry point: its CLI operates
/// on designs that already exist, and `transmog-editor-agent` is a demo binary
/// with its fixtures compiled in rather than a command that takes a brief. So
/// the only mode that runs end to end right now is [`DesignSource::Fixture`],
/// and asking for [`DesignSource::Brief`] says exactly what is missing instead
/// of quietly scoring something else.
#[derive(Debug, Clone)]
pub enum DesignSource {
    /// Score an existing `DesignDocument` RON.
    ///
    /// Useful on its own: it exercises the build and conformance half of the
    /// rubric as a regression test, with no design agent in the loop.
    Fixture(PathBuf),
    /// Have the backend design the part from the task's brief. The real eval.
    Brief,
}

/// Drives the `transmog` CLI out of a transmog checkout.
#[derive(Debug, Clone)]
pub struct TransmogBackend {
    /// Path to the transmog checkout (the directory holding its workspace
    /// `Cargo.toml`).
    pub repo: PathBuf,
    /// A pre-built `transmog` binary. Strongly preferred over building through
    /// cargo: `cargo run` writes its own progress to the stderr this harness
    /// captures, so a stage's stderr stops being only the backend's.
    pub binary: Option<PathBuf>,
    pub design: DesignSource,
}

impl TransmogBackend {
    /// A backend rooted at a transmog checkout, driven through cargo.
    #[must_use]
    pub fn new(repo: impl Into<PathBuf>, design: DesignSource) -> Self {
        Self {
            repo: repo.into(),
            binary: None,
            design,
        }
    }

    /// Uses a pre-built `transmog` binary instead of building through cargo.
    #[must_use]
    pub fn with_binary(mut self, binary: impl Into<PathBuf>) -> Self {
        self.binary = Some(binary.into());
        self
    }

    /// The program and leading arguments that invoke `transmog`.
    ///
    /// `cargo run --manifest-path <repo>/Cargo.toml` rather than setting the
    /// child's working directory to the repo: the task's own paths stay
    /// relative to where the harness was invoked, which is what a person
    /// re-running the printed command by hand would expect.
    fn invocation(&self) -> (PathBuf, Vec<String>) {
        self.binary.as_ref().map_or_else(
            || {
                (
                    PathBuf::from("cargo"),
                    vec![
                        "run".to_owned(),
                        "--quiet".to_owned(),
                        "--release".to_owned(),
                        "--manifest-path".to_owned(),
                        self.repo.join("Cargo.toml").display().to_string(),
                        "-p".to_owned(),
                        "transmog-cli".to_owned(),
                        "--bin".to_owned(),
                        "transmog".to_owned(),
                        "--".to_owned(),
                    ],
                )
            },
            |bin| (bin.clone(), Vec::new()),
        )
    }

    /// Runs one `transmog` subcommand, capturing everything it said.
    ///
    /// A non-zero exit is recorded, not raised: a backend that fails a stage
    /// is a result the rubric has an opinion about.
    fn stage(&self, name: &str, args: &[String]) -> Result<StageRun, RunError> {
        let (program, mut argv) = self.invocation();
        argv.extend_from_slice(args);
        let command = std::iter::once(program.display().to_string())
            .chain(argv.iter().cloned())
            .collect::<Vec<_>>()
            .join(" ");

        let output = Command::new(&program)
            .args(&argv)
            .output()
            .map_err(|source| RunError::Spawn {
                stage: name.to_owned(),
                program: program.display().to_string(),
                source,
            })?;

        Ok(StageRun {
            name: name.to_owned(),
            command,
            exit_code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

/// Name of the build stage, shared with the scorer's reporting.
pub const STAGE_BUILD: &str = "build-stream";
/// Schema tag `transmog build-stream` writes into `status.json`.
pub const BUILD_STATUS_SCHEMA: &str = "transmog.build.stream.v1";
/// Decision trace filename the harness looks for in the work directory.
pub const DECISION_TRACE_FILE: &str = "decisions.json";

impl Backend for TransmogBackend {
    fn name(&self) -> &str {
        "transmog"
    }

    fn run(&self, task: &Task, workdir: &Path) -> Result<RunOutcome, RunError> {
        std::fs::create_dir_all(workdir).map_err(|source| RunError::Workdir {
            path: workdir.display().to_string(),
            source,
        })?;

        let design_path = match &self.design {
            DesignSource::Fixture(path) => path.clone(),
            DesignSource::Brief => {
                return Err(RunError::CapabilityMissing {
                    backend: self.name().to_owned(),
                    capability: "design a part from a brief".to_owned(),
                    detail: format!(
                        "task {:?} needs a brief-to-DesignDocument entry point. The transmog CLI \
                         has none today: its subcommands all take geometry or a design that \
                         already exists, and transmog-editor-agent is a demo binary with its \
                         fixtures compiled in. Re-run with a fixture design to score the build \
                         and conformance criteria, or add the entry point to the backend.",
                        task.id
                    ),
                })
            }
        };

        // `transmog build-stream <design.ron> --out <workdir>/build` is the
        // one real entry point that takes a DesignDocument RON and produces
        // scoreable artifacts: it replays the design feature by feature and
        // rewrites `status.json` as it goes.
        let build_dir = workdir.join("build");
        let stages = vec![self.stage(
            STAGE_BUILD,
            &[
                "build-stream".to_owned(),
                design_path.display().to_string(),
                "--out".to_owned(),
                build_dir.display().to_string(),
            ],
        )?];

        Ok(RunOutcome {
            task_id: task.id.clone(),
            backend: self.name().to_owned(),
            workdir: workdir.to_path_buf(),
            stages,
            design_path: Some(design_path),
            build: read_build_status(&build_dir.join("status.json"))?,
            decisions: read_decision_trace(&workdir.join(DECISION_TRACE_FILE))?,
        })
    }
}

/// Reads `status.json`, or `None` if the build never wrote one.
///
/// A missing file is not an error here — a stage that died before its first
/// step leaves no status, and that is a rubric outcome rather than a harness
/// failure. A file that exists but does not parse *is* an error: that means
/// the contract between harness and backend has drifted, and guessing past it
/// would score a run against a document nobody can read.
///
/// # Errors
/// The file exists but cannot be read or parsed.
pub fn read_build_status(path: &Path) -> Result<Option<BuildStatus>, RunError> {
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(path).map_err(|source| RunError::Artifact {
        path: path.display().to_string(),
        source,
    })?;
    let status: BuildStatus =
        serde_json::from_str(&text).map_err(|source| RunError::ParseArtifact {
            path: path.display().to_string(),
            source,
        })?;
    Ok(Some(status))
}

/// Reads a decision trace, or an empty trace if the backend wrote none.
///
/// # Errors
/// The file exists but cannot be read or parsed.
pub fn read_decision_trace(path: &Path) -> Result<Vec<DecisionRecord>, RunError> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = std::fs::read_to_string(path).map_err(|source| RunError::Artifact {
        path: path.display().to_string(),
        source,
    })?;
    serde_json::from_str(&text).map_err(|source| RunError::ParseArtifact {
        path: path.display().to_string(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_invocation_names_the_real_subcommand_and_manifest() {
        let backend = TransmogBackend::new("/repo", DesignSource::Brief);
        let (program, args) = backend.invocation();
        assert_eq!(program, PathBuf::from("cargo"));
        assert!(args.contains(&"/repo/Cargo.toml".to_owned()), "{args:?}");
        assert!(args.contains(&"transmog-cli".to_owned()), "{args:?}");
        assert!(args.contains(&"transmog".to_owned()), "{args:?}");
        assert_eq!(args.last().expect("trailing separator"), "--");
    }

    #[test]
    fn a_prebuilt_binary_is_invoked_directly() {
        let backend =
            TransmogBackend::new("/repo", DesignSource::Brief).with_binary("/bin/transmog");
        let (program, args) = backend.invocation();
        assert_eq!(program, PathBuf::from("/bin/transmog"));
        assert!(args.is_empty(), "{args:?}");
    }

    #[test]
    fn designing_from_a_brief_fails_loud_rather_than_scoring_something_else() {
        let task = Task {
            id: "t".to_owned(),
            family: "machined-plate".to_owned(),
            brief: "A plate.".to_owned(),
            rubric: Vec::new(),
        };
        let backend = TransmogBackend::new("/repo", DesignSource::Brief);
        let dir = std::env::temp_dir().join("cadbench-brief-test");
        let err = backend.run(&task, &dir).expect_err("no such capability");
        assert!(matches!(err, RunError::CapabilityMissing { .. }), "{err:?}");
        assert!(err.to_string().contains("design a part from a brief"));
    }

    #[test]
    fn a_missing_status_file_is_absence_not_failure() {
        let missing = Path::new("/nonexistent/status.json");
        assert!(read_build_status(missing).expect("absence is ok").is_none());
        assert!(read_decision_trace(missing)
            .expect("absence is ok")
            .is_empty());
    }

    #[test]
    fn build_status_parses_the_real_schema() {
        // Field-for-field what `transmog build-stream` writes (see
        // crates/transmog-cli/src/build_stream.rs). Extra fields are present
        // here on purpose: the parser must tolerate them.
        let json = r#"{"schema":"transmog.build.stream.v1","source":"p.ron","total":9,"done":9,
            "complete":true,"failed":null,"opaque":[],"patches":[],"running":null,
            "updatedEpochMs":1726900000000,
            "steps":[{"index":0,"feature":1,"kind":"box","csg":null,"slow":false,"ms":1.5,
            "tris":12,"file":"step-0.json"}]}"#;
        let status: BuildStatus = serde_json::from_str(json).expect("parses");
        assert_eq!(status.schema, BUILD_STATUS_SCHEMA);
        assert_eq!(status.total, 9);
        assert!(status.complete);
        assert!(status.failed.is_none());
    }

    #[test]
    fn stage_ok_is_exit_zero_only() {
        let stage = |code| StageRun {
            name: STAGE_BUILD.to_owned(),
            command: "transmog build-stream".to_owned(),
            exit_code: code,
            stdout: String::new(),
            stderr: String::new(),
        };
        assert!(stage(Some(0)).ok());
        assert!(!stage(Some(1)).ok());
        // Killed by a signal: no code, and emphatically not a pass.
        assert!(!stage(None).ok());
    }
}
