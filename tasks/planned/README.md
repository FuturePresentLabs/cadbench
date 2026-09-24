# Planned tasks

Target specs for eval tasks in families transmog's curated library doesn't
support yet — today that's just `mounting-plate` (`Family::MountingPlate`,
"rectangular stock, one centre boss, three bores").

These files use the same schema as `tasks/*.toml` but live one directory
deeper on purpose:

- `scripts/update-badges.sh` only counts `tasks/*.toml` (`-maxdepth 1`), so
  these don't inflate the eval badge with tasks that can't actually run.
- `every_shipped_task_parses_and_has_a_sound_rubric` only reads `tasks/`
  itself, not subdirectories, so these aren't asserted against as if they
  were real, currently-passing evals.

Move a file up into `tasks/` (and drop its `# PLANNED` header comment) once
its `family` exists as a real curated family and a live run has actually
been scored against it — matching this project's own rule that a task
file is the harness's contract, not sample data.

Current planned families:

- **`bracket-l`** — a structural L-bracket, bigger and load-bearing in a
  way mounting-plate isn't (cantilevered motor mount, ribbing matters).
  Likely the same feature-based/milling generation approach as
  mounting-plate extended to a new curated shape, not a new paradigm.
- **`turned-stepped-shaft`** — a lathe-turned stepped shaft. Flagged as
  the harder one: rotationally symmetric generation (diameter steps along
  one axis) is a genuinely different shape grammar than mounting-plate's
  or bracket-l's box-or-angle-plus-features approach, and likely needs its
  own generation strategy on the backend rather than just a new curated
  shape in the existing generator.

## First-eight suite

`first-eight/` is the evaluation-first target suite specified by transmog's
`docs/OODA_DESIGN_LOOP.md`. These are immutable task contracts, deliberately
authored before the family-neutral OODA/IR loop exists:

1. four-hole mounting plate;
2. pocketed electronics base plate;
3. stepped shaft;
4. L motor bracket;
5. flanged bearing block;
6. sensor cover;
7. vented Raspberry Pi 4 enclosure;
8. an impossible sealed enclosure that must be refused.

They remain under `planned/` until a baseline run has been recorded and the
runner can route their family through the appropriate design entry point.
Moving a task into `tasks/` is promotion of an existing contract, not an
opportunity to change its brief or weaken its checks after seeing a result.
