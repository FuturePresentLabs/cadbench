//! `cadbench run <task.toml> --repo <transmog-checkout>`
//!
//! Prints the scored rubric as JSON and exits non-zero if any automated
//! criterion failed. [`Verdict::NeedsHuman`] never fails the run — see
//! [`cadbench::scorer::ScoreReport::all_automated_pass`].

use std::path::PathBuf;
use std::process::ExitCode;

use cadbench::runner::{Backend, DesignSource, RunError, TransmogBackend};
use cadbench::scorer::score;
use cadbench::task::Task;
use clap::Parser;

#[derive(Parser)]
#[command(about = "Eval harness for typed-decision-driven CAD design agents")]
struct Args {
    /// Path to a task TOML file, e.g. tasks/mounting-plate-v1.toml.
    task: PathBuf,

    /// Path to a transmog checkout (the directory holding its workspace Cargo.toml).
    #[arg(long)]
    repo: PathBuf,

    /// A pre-built `transmog` binary. Skips `cargo run` when set.
    #[arg(long)]
    binary: Option<PathBuf>,

    /// Score an existing DesignDocument RON instead of asking the backend to
    /// design from the task's brief. The only mode that runs end to end
    /// today — transmog has no brief-to-DesignDocument entry point yet (see
    /// runner::DesignSource docs). Omit once that exists.
    #[arg(long)]
    fixture: Option<PathBuf>,

    /// Directory to write run artifacts into.
    #[arg(long, default_value = "cadbench-run")]
    out: PathBuf,
}

fn main() -> ExitCode {
    let args = Args::parse();

    let text = match std::fs::read_to_string(&args.task) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("error: reading {}: {error}", args.task.display());
            return ExitCode::FAILURE;
        }
    };
    let task: Task = match toml::from_str(&text) {
        Ok(task) => task,
        Err(error) => {
            eprintln!("error: parsing {}: {error}", args.task.display());
            return ExitCode::FAILURE;
        }
    };

    let design = match args.fixture {
        Some(path) => DesignSource::Fixture(path),
        None => DesignSource::Brief,
    };
    let mut backend = TransmogBackend::new(args.repo, design);
    if let Some(binary) = args.binary {
        backend = backend.with_binary(binary);
    }

    let outcome = match backend.run(&task, &args.out) {
        Ok(outcome) => outcome,
        Err(error) => {
            report_run_error(&error);
            return ExitCode::FAILURE;
        }
    };

    let report = score(&task, &outcome);
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

    if report.all_automated_pass() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn report_run_error(error: &RunError) {
    eprintln!("error: {error}");
    if let RunError::CapabilityMissing { detail, .. } = error {
        eprintln!("{detail}");
    }
}
