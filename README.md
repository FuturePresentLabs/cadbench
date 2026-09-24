# cadbench

[![license](https://img.shields.io/badge/license-AGPL--3.0--or--later-blue.svg)](#license)
[![tests](https://img.shields.io/badge/tests-27%20passing-brightgreen.svg)](#status)
[![evals](https://img.shields.io/badge/evals-28-orange.svg)](#tasks)
[![status](https://img.shields.io/badge/status-runs%20end%20to%20end-success.svg)](#status)

*(Badge numbers are generated — run `scripts/update-badges.sh` after a test or task count changes; don't hand-edit them.)*

Eval harness for typed-decision-driven CAD design agents. Sibling to
[`pcbbench`](../pcbbench) — same shape, different domain. A task names a
design brief and a rubric. A backend attempts it as a subprocess. The
harness scores whatever artifacts land on disk.

Deliberately CAD-kernel-agnostic: this crate never links a backend as a
dependency, only spawns one and reads its output. "Score a different tool"
is a new [`runner::Backend`] impl, not a rewrite.

## Status

**Runs end to end**, one known shortcut called out below — the same
honesty pcbbench and legion-of-bom hold themselves to. 22 unit tests, 28
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

## Writing tasks

[GUIDELINES.md](GUIDELINES.md) sets the authoring rules. G1–G6 are enforced
by a test: each task's expected numbers show their hand derivation, briefs
never give away an answer's key, and every family has a task it must refuse.
Shape-driven tasks (`family = "svg-part"`) take an `[input]` table (an SVG,
a height and a material) and are scored on volume, bounds, true-surface
STEP, cut plan and the decisions the brief settles.

## Usage

```bash
# The real eval: the backend designs the part from the task's brief.
cargo run -- tasks/mounting-plate-v1.toml --repo /path/to/transmog

# Run every promoted task through eval's shared suite runner.
cargo run -- --all --tasks-dir tasks --repo /path/to/transmog --out cadbench-results

# ...with the design stage calling the real decision gateway. Needs
# BIFROST_API_KEY in the environment; off by default, because a benchmark
# that reaches the network unasked produces numbers nobody can reproduce.
cargo run -- tasks/mounting-plate-v1.toml --repo /path/to/transmog --live

# Pin both model roles independently. This backend consumes the RLCD model;
# the LLM identity is retained for comparable cross-harness provenance.
cargo run -- tasks/mounting-plate-v1.toml --repo /path/to/transmog --live \
    --llm-model anthropic/claude-sonnet-4.5 --rlcd-model fpl/decide

# Build-and-conformance regression only, no design agent in the loop.
cargo run -- tasks/mounting-plate-v1.toml \
    --repo /path/to/transmog \
    --fixture /path/to/some-design.ron
```

The shared `eval::ModelSelection` owns these flags across PCB/CAD/CAM/DFM.
`EVAL_LLM_MODEL` and `EVAL_RLCD_MODEL` are their environment equivalents.

Prints the scored rubric as JSON. Exits non-zero if any *automated*
criterion failed — `Verdict::NeedsHuman` (the `subjective` checks) never
fails the run; it's reported separately as still needing a person.

`--all` requires `--live`. Recorded responses belong to one captured brief,
and one `--fixture` design cannot represent a multi-prompt model benchmark;
CADBench refuses both rather than publishing duplicated artifacts as a model
score. Every task gets an isolated child of `--out`, individual backend errors
do not stop the suite, and the shared `eval.suite-report.v1` aggregate is
written to `suite-report.json`. Nested `tasks/planned/` contracts are not run
until they are promoted.

Pass `--binary /path/to/transmog` instead of relying on `cargo run
--release` under the hood if you already have one built — a cargo build
mixes its own progress into the stderr this harness captures.

## Tasks

Twenty-eight, all under `tasks/`, one part family (`mounting-plate` —
Transmog's only curated family today; the geometric template is fixed, so
every task varies the *brief*, not the shape). Grouped by what each one
stresses:

**Baseline & positive controls** — golden-path briefs that should always
pass `confidence`. If one of these ever fails, the harness or the backend
regressed, not the brief.
- `mounting-plate-v1` — the original baseline (one underspecified tolerance
  on purpose, see below).
- `mounting-plate-unambiguous-v1`, `mounting-plate-7075-structural-large-v1`,
  `mounting-plate-instrument-panel-small-v1`, `mounting-plate-prototype-loose-v1`,
  `mounting-plate-galvanized-outdoor-v1`, `mounting-plate-delrin-lightweight-v1`
  — loose tolerances, clear intent, different scales and materials each
  time so "positive control" doesn't collapse into one shape being tested
  five times.

**Tight tolerance, source stated** — hard numbers, but the *why* is given.
Should still clear `confidence`.
- `mounting-plate-cast-iron-machine-base-v1` (mating housing's own spec),
  `mounting-plate-titanium-aerospace-v1` (connector datasheet spec),
  `mounting-plate-304-stainless-tight-budget-v1` (equipment manufacturer's
  spec), `mounting-plate-7075-robotics-arm-v1` (joint repeatability spec).

**Ambiguity stress cases** — a real reason for a tight tolerance, with the
one number that matters never given. Expected to fail `confidence`, each
for a different underlying reason.
- `mounting-plate-tight-tolerance-v1` (stack-up budget never stated),
  `mounting-plate-thermal-mismatch-tight-v1` (thermal expansion mismatch,
  no temperature range or CTE given), `mounting-plate-customer-mating-part-v1`
  (tolerance pending an external party's drawing).

**Brief-length stress cases** — does the agent have enough signal, in
either direction.
- `mounting-plate-terse-minimal-v1`, `mounting-plate-c360-brass-terse-v1` —
  one sentence, almost nothing to go on.
- `mounting-plate-verbose-overspec-v1` — heavy logistics/paperwork noise
  around otherwise-unambiguous technical content.
- `mounting-plate-verbose-ambiguous-v1` — verbose *and* still never states
  the one tolerance that matters; the compound case.

**Material variety** — chosen where the material itself carries a design
reason (corrosion, weight, damping, cost), not a find-replace.
- `mounting-plate-304-stainless-marine-v1` (salt-spray corrosion resistance).

**Capability edge** — past what the curated template can express.
- `mounting-plate-dual-boss-request-v1` — asks for two bosses; the template
  is one. Expected to surface as a failed or capability-missing stage, not
  a silently substituted one-boss part. The rubric says so honestly rather
  than assuming a `conforms` pass.

**Grounded in a real standard** — tolerance language pulled from
[ISO 286](https://en.wikipedia.org/wiki/ISO_286)/ANSI B4.1's limits-and-fits
system rather than invented numbers, so "ambiguous" and "unambiguous" are
judged against how a real GD&T-literate engineer would actually write a
fit callout, not against a made-up bar.
- `mounting-plate-iso-h7g6-clearance-v1` — positive control: the locating
  dowel's fit is fully specified (`H7/g6`, hand-assembled, removable).
- `mounting-plate-iso-fit-unspecified-v1` — ambiguity stress case: brief
  wants "removable but won't walk under vibration," which sits between a
  clearance fit (`H7/g6`) and a light interference fit (`H7/n6`) with no
  stated preference — a real, common engineering ambiguity, not a
  contrived one.

## Task format

TOML, one file per task under `tasks/`. Each `[[rubric]]` entry is one
criterion:

Brief-driven tasks may include an evaluator-only oracle:

```toml
[input]
expected_stock_mm = [100.0, 60.0, 20.0]
expected_material = "304 stainless steel"

[[rubric]]
id = "starting-stock"
description = "the generative model extracted the stated stock"
kind = "starting_stock"
```

The harness sends only `brief` to the backend's generative extraction stage.
It scores the resulting `starting-stock.json` against `[input]`, then fixes
that stock before RLCD makes bounded geometry decisions. The oracle is never
included in either model request.

- `stages_pass` — every stage that ran exited 0.
- `starting_stock` — extracted dimensions and material match the task oracle.
- `product_inputs` — board, material, ingress target, fastener, and ISO 273
  clearance series extracted from the brief match the public rubric.
- `min_decision_confidence` (`threshold`) — every recorded typed decision
  cleared the bar. No decisions recorded fails this rather than passing it
  vacuously.
- `conforms` — see the known gap above.
- `subjective` — not automated; reported as needing a human rather than
  guessing, mirroring pcbbench's "vibe" criterion.
- `lua` (`script`, `expected`) — a sandboxed product-specific composition of
  Rust-owned predicates. Paths are relative to the task. The backend writes
  `eval-facts.json` using schema `cadbench.eval-facts.v1`; checked-in expected
  values use `cadbench.lua-check-input.v1`. Public checks never link or name a
  backend-private implementation.

Lua is intentionally not a geometry or standards engine. Its only host API is:

- `check.require_role(role)` — exactly one part has the controlled semantic
  role;
- `check.require_predicate(kind, roles)` — Rust resolves each role to one
  stable part ID and requires an exact predicate fact;
- `check.require_expected_roles()` and
  `check.require_expected_predicates()` — apply the checked-in JSON contract.

Only the Lua table, string, and math libraries are loaded, execution is capped
at 100,000 instructions, and the script has no filesystem, process, network,
module-loading, raw-geometry, or generated-name access. Missing facts,
ambiguous roles, dangling IDs, schema drift, and absent predicates fail
loudly. See `tasks/planned/checks/assembly-v1.lua` and its adjacent JSON
contracts for the first vertical slice.

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

## Alternatives

Other benchmarks in this space, for context and future comparison — not
wired up here, listed so we don't reinvent a comparison that already
exists:

- **[Hephaestus-CCX](https://arxiv.org/abs/2605.17448)** — the closest real
  analog: 50 single/multi-part engineering briefs, each judged by typed
  pass/fail requirement checkers against real CalculiX FEA output (stress,
  displacement, buckling, modal), in an iterative
  brief→CadQuery→STEP→FEA-feedback loop. Best reported result: 60.5% mean
  requirement satisfaction, 9/50 strict passes, after ~70 min/item of
  iteration. The paper states the benchmark is released, but no clean
  harness URL was confirmed as of 2026-09-22 — a large (89GB) logged-runs
  dataset exists on HuggingFace under a matching namespace, but that reads
  as experiment output, not the raw brief/checker bundle. Planned: measure
  `transmog` against these briefs once the harness is actually reachable —
  the nearest thing to an apples-to-apples comparison cadbench has.
- **[MUSE](https://arxiv.org/pdf/2605.28579)** — multi-part B-Rep assemblies
  scored on functionality, manufacturability, and assemblability by a
  rubric-based VLM judge rather than geometric similarity alone. Public
  leaderboard, dataset, and code.
- **[Text2CAD-Bench](https://arxiv.org/pdf/2605.18430)** — 600
  human-curated examples across 4 difficulty levels.
- **[CADTests](https://arxiv.org/pdf/2605.07807)** — executable tests
  verifying a generated model's geometric/topological requirements are
  actually satisfied, not just visually plausible.
- **BenchCAD** — 17,900 CadQuery programs, 106 part families; pure
  code-generation, not a full brief→artifact→verdict pipeline like this
  one.

## License

AGPL-3.0-or-later, matching `pcbbench` and `legion-of-bom`.
