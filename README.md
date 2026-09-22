# cadbench

Eval harness for typed-decision-driven CAD design agents. Sibling to
[`pcbbench`](../pcbbench), same shape, different domain: a task names a
design brief and a rubric; a backend attempts it as a subprocess; the
harness scores whatever artifacts land on disk. Deliberately CAD-kernel-
agnostic — this crate never links a backend as a dependency, only spawns
one and reads its output, so "score a different tool" is a new
[`runner::Backend`] impl, not a rewrite.

**Status: early scaffold**, same honesty pcbbench and legion-of-bom hold
themselves to. The harness itself works and is tested (17 unit tests), but
it cannot run a real end-to-end eval yet — see the gap below before you
expect a score.

## The gap that matters right now

`transmog` (the first backend) has **no brief-to-`DesignDocument` entry
point**. Its CLI operates on designs that already exist; `transmog-editor-agent`
is a demo binary with its fixtures compiled in, not a command that takes a
brief. So `--fixture <design.ron>` is the only mode that runs today — it
exercises the build and conformance half of the rubric as a real regression
test, with no design agent in the loop at all. Asking for the real mode
(brief in, design out) fails loud with `CapabilityMissing` naming exactly
what's missing, rather than silently scoring something else.

A second, smaller gap: the `conforms` rubric check is a **known shortcut**
right now — it checks that `transmog build-stream` completed without the
kernel refusing a boolean, not that the geometry actually satisfies its own
declared tolerances. The real check
(`transmog-editor-agent::contract::ConformanceReport`) exists and is fully
tested, but is reachable from no CLI. See the doc comment on
`scorer::conforms_verdict` for the exact fix once that wrapper lands.

## Usage

```bash
cargo run -- tasks/mounting-plate-v1.toml \
    --repo /path/to/transmog \
    --fixture /path/to/some-design.ron
```

Prints the scored rubric as JSON; exits non-zero if any *automated*
criterion failed (`Verdict::NeedsHuman` — the `subjective` checks — never
fails the run; it's reported separately as still needing a person).

Pass `--binary /path/to/transmog` instead of relying on `cargo run
--release` under the hood if you already have one built — a cargo-built
binary would mix its own build progress into the stderr this harness
captures.

## Task format

TOML, one file per task under `tasks/`. See `tasks/mounting-plate-v1.toml`
for the real, tested example. Each `[[rubric]]` entry is one criterion:

- `stages_pass` — every stage that ran exited 0.
- `min_decision_confidence` (`threshold`) — every recorded typed decision
  cleared the bar. No decisions recorded fails this rather than passing it
  vacuously.
- `conforms` — see the shortcut noted above.
- `subjective` — not automated; the harness reports it as needing a human
  rather than guessing, mirroring pcbbench's "vibe" criterion.

## Relationship to the rest of this family

- **[`pcbbench`](../pcbbench)** — the PCB-design sibling this was modeled
  on directly; same architecture, same `DecisionRecord` trace shape
  (kept field-for-field identical on purpose, see `runner::DecisionRecord`).
- **`dfmbench`** (not started) — judges whether a design's material/process
  choice is sound, and whether an otherwise-valid design is actually
  manufacturable. Explicitly out of scope here: cadbench only asks "does
  the modeled shape match the brief," never "was this the right process or
  material" — that's DFMBench's whole reason to exist.
- **CAMBench** (not started) — given a design with its process already
  settled, can it actually be machined/printed/molded well. Also out of
  scope here.

## License

AGPL-3.0-or-later, matching `pcbbench` and `legion-of-bom`.
