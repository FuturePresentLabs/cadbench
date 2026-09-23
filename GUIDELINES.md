# Writing a task

A task is a claim about what a correct result looks like. These rules make
that claim checkable. G1–G6 are enforced by
`task::tests::every_shipped_task_follows_the_authoring_guidelines`, so a
task that breaks one fails `cargo test`. The rest are review rules.

## Enforced

**G1. Inputs load.** A task's `[input]` parses as the harness's input type,
and every file it names exists. Paths are relative to the task file.

**G2. Anything built is checked against a number.** A task that builds a
part carries at least one `volume_mm3` or `bounds_mm` criterion. Stages
passing and confident decisions do not show that the part is right.

**G3. Every expected number shows its working, and tolerances are tight.**
`volume_mm3`, `cut_plan` and `true_surfaces` carry a `derivation`: the hand
arithmetic the number came from, so anyone can audit it without running
anything. An expected value is never read back off the backend under test.
Tolerances: volume within 1e-4 of the expected value, bounds and cut length
within 0.05 mm. A looser tolerance lets a wrong answer through, and a wrong
answer is what the task is there to catch.

**G4. The brief states intent and never gives away the answer.** When the
brief settles a decision, assert the answer with a `decision` criterion,
not only confidence, because a model can be sure and still wrong. The
brief must not contain the answer's machine key (`counterbore_m3`,
`hollow_2p0mm`). Say "the screw heads end up below the top face", not the
key.

**G5. At most one `subjective` criterion, and never the only one.**

**G6. Every family that builds parts has a refusal task.** Some tasks must
be ones whose honest answer is "this cannot be made as asked": text in a
drawing, a slot narrower than the jet. Without them, a backend that quietly
substitutes something makeable scores the same as one that refuses.

## Review rules

- **Headline numbers are live.** Report `--live` runs, repeated. A task
  that passes once and fails once is unstable, and that instability is the
  finding. Offline runs are a regression check, not a score: every
  undecided question is traced as `default` at confidence 0.0, so an
  offline run cannot pass `min_decision_confidence`, by design.
- **Key decisions by prefix when numbering is incidental.** Use `holes_*`
  when every hole group must get the same answer. Name `cutout_0` only when
  the task depends on which shape it is, and say which one in the
  description.
- **Fixtures are inputs, checked in.** Never generate a fixture with the
  backend you are grading.
- **A real part beats a synthetic one.** When a task models something that
  exists (a board, a case), take its datums from the maker's published
  mechanical drawing or design files, and cite the source in the task.
