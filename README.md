# cadbench

[![license](https://img.shields.io/badge/license-AGPL--3.0--or--later-blue.svg)](#license)
[![tests](https://img.shields.io/badge/tests-20%20passing-brightgreen.svg)](#status)
[![status](https://img.shields.io/badge/status-runs%20end%20to%20end-success.svg)](#status)

Eval harness for typed-decision-driven CAD design agents. Sibling to
[`pcbbench`](../pcbbench) — same shape, different domain. A task names a
design brief and a rubric. A backend attempts it as a subprocess. The
harness scores whatever artifacts land on disk.

Deliberately CAD-kernel-agnostic: this crate never links a backend as a
dependency, only spawns one and reads its output. "Score a different tool"
is a new [`runner::Backend`] impl, not a rewrite.

## Status

**Runs end to end**, one known shortcut called out below — the same
honesty pcbbench and legion-of-bom hold themselves to. 20 unit tests, three
tasks. `transmog` (today's only backend) grew the brief-to-`DesignDocument`
entry point this harness was built to drive, so the default mode — no
`--fixture` — is the real eval, not a fixture-only regression.

A scored run of `tasks/mounting-plate-v1.toml`, design stage `--live`
against the real decision gateway:

| criterion | verdict | detail |
|---|---|---|
| `stages` | Pass | 2 stage(s), all exited 0 |
| `confidence` | **Fail** | 3 decisions, worst confidence 0.67 vs threshold 0.70 |
| `conforms` | Pass | build completed (9/9 steps) — proxy check, see below |
| `vibe` | NeedsHuman | not automated |

That failing criterion is the harness working, not breaking. Two of the
three typed decisions came back at 0.95; the third — which positional
tolerance zone to put on the locating bore — came back at 0.57-0.67 across
three runs, consistently under the bar. Nothing in the brief says how
tightly that bore has to be held, so the model has little to go on and its
confidence says so. Recorded mode (drop `--live`) passes every automated
criterion, which is what makes it the reproducible reference run.

`--fixture <design.ron>` stays useful on its own: it skips the design stage
entirely, exercising the build and conformance half of the rubric as a
regression test with no agent in the loop.

### The known gap

`conforms` is a **known shortcut**: it checks that `transmog build-stream`
completed without the kernel refusing a boolean, not that the geometry
actually satisfies its own declared tolerances. The real check
(`transmog-editor-agent::contract::ConformanceReport`) exists and is fully
tested, but is reachable from no CLI yet. See `scorer::conforms_verdict`'s
doc comment for the exact fix once that wrapper lands.

## Usage

```bash
# The real eval: the backend designs the part from the task's brief.
cargo run -- tasks/mounting-plate-v1.toml --repo /path/to/transmog

# ...with the design stage calling the real decision gateway. Needs
# BIFROST_API_KEY in the environment; off by default, because a benchmark
# that reaches the network unasked produces numbers nobody can reproduce.
cargo run -- tasks/mounting-plate-v1.toml --repo /path/to/transmog --live

# Build-and-conformance regression only, no design agent in the loop.
cargo run -- tasks/mounting-plate-v1.toml \
    --repo /path/to/transmog \
    --fixture /path/to/some-design.ron
```

Prints the scored rubric as JSON. Exits non-zero if any *automated*
criterion failed — `Verdict::NeedsHuman` (the `subjective` checks) never
fails the run; it's reported separately as still needing a person.

Pass `--binary /path/to/transmog` instead of relying on `cargo run
--release` under the hood if you already have one built — a cargo build
mixes its own progress into the stderr this harness captures.

## Tasks

Three, all under `tasks/`, one part family (`mounting-plate`) with
different briefs stressing different parts of the rubric:

- **`mounting-plate-v1`** — the baseline: enough detail to build, one
  underspecified tolerance to make the confidence criterion mean something.
- **`mounting-plate-tight-tolerance-v1`** — a precision stack-up with no
  stated tolerance budget. A deliberately hard case; expected to fail
  `confidence` until the brief (or the model) resolves the ambiguity.
- **`mounting-plate-unambiguous-v1`** — a golden-path positive control.
  Every dimension and tolerance has one obvious answer. If this one ever
  fails, the harness or the backend regressed — not the brief.

## Task format

TOML, one file per task under `tasks/`. Each `[[rubric]]` entry is one
criterion:

- `stages_pass` — every stage that ran exited 0.
- `min_decision_confidence` (`threshold`) — every recorded typed decision
  cleared the bar. No decisions recorded fails this rather than passing it
  vacuously.
- `conforms` — see the known gap above.
- `subjective` — not automated; reported as needing a human rather than
  guessing, mirroring pcbbench's "vibe" criterion.

## Family

- **[`pcbbench`](../pcbbench)** — the PCB-design sibling this was modeled
  on; same architecture, same `DecisionRecord` trace shape (kept
  field-for-field identical on purpose).
- **`dfmbench`** (not started) — judges whether a design's material/process
  choice is sound and whether an otherwise-valid design is actually
  manufacturable. Out of scope here: cadbench only asks "does the modeled
  shape match the brief," never "was this the right process or material."
- **CAMBench** (not started) — given a design with its process already
  settled, can it actually be machined/printed/molded well. Also out of
  scope here.

## License

AGPL-3.0-or-later, matching `pcbbench` and `legion-of-bom`.
