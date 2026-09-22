#!/usr/bin/env bash
# Regenerates README.md's test-count and eval-count badges from ground
# truth -- run this instead of hand-editing the badge numbers, so they
# can't drift from reality the way manually maintained ones always do.
#
# Fails loud (set -e + pipefail) if any test fails: a badge update is
# refused, not silently computed from a broken run.
set -euo pipefail
cd "$(dirname "$0")/.."

output="$(cargo test 2>&1)"
tests="$(grep -oE '^test result: ok\. [0-9]+ passed' <<<"$output" \
    | grep -oE '[0-9]+' \
    | awk '{s+=$1} END {print s+0}')"
evals="$(find tasks -maxdepth 1 -name '*.toml' | wc -l | tr -d ' ')"

if [ "$tests" -eq 0 ]; then
    echo "update-badges: found 0 passing tests -- refusing to write a bogus badge" >&2
    echo "$output" >&2
    exit 1
fi
if [ "$evals" -eq 0 ]; then
    echo "update-badges: found 0 task files under tasks/ -- refusing to write a bogus badge" >&2
    exit 1
fi

sed -E "s#(badge/tests-)[0-9]+(%20passing)#\\1${tests}\\2#" README.md > README.md.tmp
mv README.md.tmp README.md
sed -E "s#(badge/evals-)[0-9]+#\\1${evals}#" README.md > README.md.tmp
mv README.md.tmp README.md

echo "tests: ${tests}, evals: ${evals}"
