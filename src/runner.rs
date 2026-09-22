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
/// [`DesignSource::Brief`] is the real eval and now runs end to end: transmog
/// grew the entry point it needs, `transmog spec <family> --brief ... --out ...
/// --trace ...`, which writes both artifacts this harness goes looking for.
/// [`DesignSource::Fixture`] stays because it is useful on its own — it
/// exercises the build and conformance half of the rubric with no design agent
/// in the loop at all, which is what you want when the question is "did the
/// kernel regress" rather than "did the agent design well".
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
    /// Pass `--live` to `transmog spec`, so the design stage calls the real
    /// decision gateway instead of replaying recorded responses.
    ///
    /// Off by default because a benchmark that silently reaches the network is
    /// a benchmark whose numbers nobody can reproduce. With it on, the backend
    /// needs `BIFROST_API_KEY` in its environment and `transmog spec` fails
    /// loudly without one — which surfaces here as a failed stage, not as a
    /// quiet fallback to the recording.
    pub live: bool,
}

impl TransmogBackend {
    /// A backend rooted at a transmog checkout, driven through cargo.
    #[must_use]
    pub fn new(repo: impl Into<PathBuf>, design: DesignSource) -> Self {
        Self {
            repo: repo.into(),
            binary: None,
            design,
            live: false,
        }
    }

    /// Uses a pre-built `transmog` binary instead of building through cargo.
    #[must_use]
    pub fn with_binary(mut self, binary: impl Into<PathBuf>) -> Self {
        self.binary = Some(binary.into());
        self
    }

    /// Runs the design stage against the real decision gateway.
    #[must_use]
    pub fn live(mut self, live: bool) -> Self {
        self.live = live;
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

/// Name of the design stage, which turns the task's brief into a document.
pub const STAGE_SPEC: &str = "spec";
/// Name of the build stage, shared with the scorer's reporting.
pub const STAGE_BUILD: &str = "build-stream";
/// Filename the design stage writes its `DesignDocument` RON to.
pub const DESIGN_FILE: &str = "design.ron";
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

        let mut stages = Vec::new();

        // `transmog spec <family> --brief <brief> --out <design> --trace
        // <decisions>` designs the part. The task's `family` goes through
        // untouched: which families exist is the backend's business, and a
        // family it does not know is a failed stage with the backend's own
        // error in it rather than something this harness has an opinion about.
        let design_path = match &self.design {
            DesignSource::Fixture(path) => path.clone(),
            DesignSource::Brief => {
                let design = workdir.join(DESIGN_FILE);
                let mut args = vec![
                    "spec".to_owned(),
                    task.family.clone(),
                    "--brief".to_owned(),
                    task.brief.clone(),
                    "--out".to_owned(),
                    design.display().to_string(),
                    "--trace".to_owned(),
                    workdir.join(DECISION_TRACE_FILE).display().to_string(),
                ];
                if self.live {
                    args.push("--live".to_owned());
                }
                stages.push(self.stage(STAGE_SPEC, &args)?);
                design
            }
        };

        // `transmog build-stream <design.ron> --out <workdir>/build` is the
        // one real entry point that takes a DesignDocument RON and produces
        // scoreable artifacts: it replays the design feature by feature and
        // rewrites `status.json` as it goes.
        //
        // It runs even when the design stage failed, and that is deliberate: a
        // build that cannot find the document records a second failed stage
        // with the reason in it, which is more use to whoever reads the result
        // than an early return that says nothing about what the build would
        // have done. Both are already failures the rubric counts.
        let build_dir = workdir.join("build");
        stages.push(self.stage(
            STAGE_BUILD,
            &[
                "build-stream".to_owned(),
                design_path.display().to_string(),
                "--out".to_owned(),
                build_dir.display().to_string(),
            ],
        )?);

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

    fn brief_task() -> Task {
        Task {
            id: "t".to_owned(),
            family: "mounting-plate".to_owned(),
            brief: "A plate with a boss.".to_owned(),
            rubric: Vec::new(),
        }
    }

    /// The design stage must invoke the backend's real entry point, with the
    /// task's own family and brief and with both output paths inside the work
    /// directory the harness then reads.
    ///
    /// Driven through a `true`-shaped stand-in binary so the assertion is about
    /// the command line this harness builds, not about transmog being present.
    #[test]
    fn designing_from_a_brief_invokes_transmog_spec_with_both_artifacts() {
        let dir = std::env::temp_dir().join("cadbench-brief-invocation");
        let _ = std::fs::remove_dir_all(&dir);
        let backend =
            TransmogBackend::new("/repo", DesignSource::Brief).with_binary("/usr/bin/true");

        let outcome = backend.run(&brief_task(), &dir).expect("stages run");
        let spec = outcome
            .stages
            .iter()
            .find(|s| s.name == STAGE_SPEC)
            .expect("a design stage ran");

        assert!(spec.command.contains(" spec "), "{}", spec.command);
        assert!(spec.command.contains("mounting-plate"), "{}", spec.command);
        assert!(
            spec.command.contains("--brief A plate with a boss."),
            "{}",
            spec.command
        );
        assert!(
            spec.command.contains(&dir.join(DESIGN_FILE).display().to_string()),
            "the design must land in the work directory: {}",
            spec.command
        );
        assert!(
            spec.command
                .contains(&dir.join(DECISION_TRACE_FILE).display().to_string()),
            "the trace must land where read_decision_trace looks: {}",
            spec.command
        );
        // Recorded by default: a benchmark must not reach the network unasked.
        assert!(!spec.command.contains("--live"), "{}", spec.command);

        // The design stage comes first; the build cannot precede the document.
        assert_eq!(outcome.stages[0].name, STAGE_SPEC);
        assert_eq!(outcome.stages[1].name, STAGE_BUILD);
        assert_eq!(
            outcome.design_path.as_deref(),
            Some(dir.join(DESIGN_FILE).as_path())
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn live_is_opt_in_and_reaches_the_design_stage() {
        let dir = std::env::temp_dir().join("cadbench-brief-live");
        let _ = std::fs::remove_dir_all(&dir);
        let backend = TransmogBackend::new("/repo", DesignSource::Brief)
            .with_binary("/usr/bin/true")
            .live(true);

        let outcome = backend.run(&brief_task(), &dir).expect("stages run");
        let spec = outcome
            .stages
            .iter()
            .find(|s| s.name == STAGE_SPEC)
            .expect("a design stage ran");
        assert!(spec.command.ends_with("--live"), "{}", spec.command);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A fixture run must not invoke the design stage at all — there is nothing
    /// to design, and a spurious stage would drag `stages_pass` down with it.
    #[test]
    fn a_fixture_run_does_not_run_the_design_stage() {
        let dir = std::env::temp_dir().join("cadbench-fixture-no-spec");
        let _ = std::fs::remove_dir_all(&dir);
        let backend = TransmogBackend::new("/repo", DesignSource::Fixture("/given.ron".into()))
            .with_binary("/usr/bin/true");

        let outcome = backend.run(&brief_task(), &dir).expect("stages run");
        assert_eq!(outcome.stages.len(), 1);
        assert_eq!(outcome.stages[0].name, STAGE_BUILD);
        assert_eq!(
            outcome.design_path.as_deref(),
            Some(Path::new("/given.ron"))
        );

        let _ = std::fs::remove_dir_all(&dir);
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
