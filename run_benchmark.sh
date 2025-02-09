#!/usr/bin/env bash
set -eo pipefail

BENCH_CMD="${BENCH_CMD:-"cargo bench"}"
BENCH_ARGS="${BENCH_ARGS:-"--noplot --baseline-lenient=__non_existing_name"}"

run() {
    set -e
    echo "$" "$@" >&2
    "$@"
}

commit="$(git rev-parse HEAD)"
git_status="$(git status --untracked-files=no --porcelain)"
if [[ -n "$git_status" ]]; then
    commit+="-dirty"
fi

echo "CPU: $(lscpu -p=MODELNAME | tail -n1)"
echo "Commit: $commit"
echo
echo "### Throughput (aligned)"
echo
run $BENCH_CMD --bench=validate_utf8 -- $BENCH_ARGS throughput-aligned

echo
echo "### Throughput (unaligned)"
echo
run $BENCH_CMD --bench=validate_utf8 -- $BENCH_ARGS throughput-unaligned

echo
echo "### Latency"
echo
run $BENCH_CMD --bench=validate_utf8 -- $BENCH_ARGS latency
