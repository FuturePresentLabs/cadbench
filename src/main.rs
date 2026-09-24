//! `cadbench run <task.toml> --repo <transmog-checkout>`
//!
//! Prints the scored rubric as JSON and exits non-zero if any automated
//! criterion failed. [`Verdict::NeedsHuman`] never fails the run — see
//! [`cadbench::scorer::ScoreReport::all_automated_pass`].

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context as _, Result};
use cadbench::runner::{Backend, DesignSource, RunError, TransmogBackend};
use cadbench::scorer::score;
use clap::Parser;
use eval::{ModelSelection, ScoreReport};

#[derive(Parser)]
#[command(about = "Eval harness for typed-decision-driven CAD design agents")]
struct Args {
    /// Path to a task TOML file, e.g. tasks/mounting-plate-v1.toml.
    task: Option<PathBuf>,

    /// Run every promoted `.toml` task directly under `--tasks-dir`.
    #[arg(long, conflicts_with = "task")]
    all: bool,

    /// Promoted task directory used by `--all`; nested planned tasks are skipped.
    #[arg(long, default_value = "tasks")]
    tasks_dir: PathBuf,

    /// Path to a transmog checkout (the directory holding its workspace Cargo.toml).
    #[arg(long)]
    repo: PathBuf,

    /// A pre-built `transmog` binary. Skips `cargo run` when set.
    #[arg(long)]
    binary: Option<PathBuf>,

    /// Score an existing DesignDocument RON instead of asking the backend to
    /// design from the task's brief. Skips the design stage entirely, which
    /// makes it a build-and-conformance regression test with no agent in the
    /// loop. Omit it for the real eval.
    #[arg(long)]
    fixture: Option<PathBuf>,

    /// Let the design stage call the real decision gateway instead of
    /// replaying recorded responses. The backend needs BIFROST_API_KEY.
    ///
    /// Off by default: a benchmark that reaches the network unasked produces
    /// numbers nobody else can reproduce.
    #[arg(long)]
    live: bool,

    /// Independent outer-LLM and RLCD model identities.
    #[command(flatten)]
    models: ModelSelection,

    /// Directory to write run artifacts into.
    #[arg(long, default_value = "cadbench-run")]
    out: PathBuf,
}

fn main() -> ExitCode {
    let args = Args::parse();

    if args.all {
        return run_suite(&args);
    }
    let Some(task) = args.task.as_deref() else {
        eprintln!("error: provide a task TOML or pass --all");
        return ExitCode::FAILURE;
    };

    match run_one(task, &args.out, &args) {
        Ok(report) => {
            print_report(&report);
            if report.all_automated_pass() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run_suite(args: &Args) -> ExitCode {
    let suite = eval::run_all(&args.tasks_dir, &args.out, |task, workdir| {
        run_one(task, workdir, args)
    });
    let report = match suite {
        Ok(report) => report,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::FAILURE;
        }
    };
    println!(
        "cadbench suite: {}/{} passed; {} failed; {} harness error(s); {} need human review",
        report.passed, report.total, report.failed, report.errors, report.needs_human
    );
    println!("report -> {}", args.out.join("suite-report.json").display());
    if report.all_automated_pass() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn run_one(task_path: &std::path::Path, out: &std::path::Path, args: &Args) -> Result<ScoreReport> {
    let task = cadbench::task::load(task_path)
        .with_context(|| format!("loading {}", task_path.display()))?;

    let design = match &args.fixture {
        Some(path) => DesignSource::Fixture(path.clone()),
        None => DesignSource::Brief,
    };
    let mut backend = TransmogBackend::new(&args.repo, design)
        .live(args.live)
        .with_models(args.models.clone());
    if let Some(binary) = &args.binary {
        backend = backend.with_binary(binary);
    }

    let outcome = backend
        .run(&task, out)
        .map_err(|error| anyhow::anyhow!(run_error_text(&error)))?;
    Ok(score(&task, &outcome))
}

fn print_report(report: &ScoreReport) {
    println!(
        "{}",
        serde_json::to_string_pretty(&report).expect("ScoreReport always serializes")
    );

    if !report.needs_human().is_empty() {
        eprintln!(
            "note: criteria still need a human: {}",
            report.needs_human().join(", ")
        );
    }
}

fn run_error_text(error: &RunError) -> String {
    let mut text = error.to_string();
    if let RunError::CapabilityMissing { detail, .. } = error {
        text.push_str(": ");
        text.push_str(detail);
    }
    text
}
