#!/usr/bin/env bash
# bench-history.sh — Run all criterion benchmarks and append results to CSV.
#
# Usage:
#   ./scripts/bench-history.sh           # run all benches
#   ./scripts/bench-history.sh compose   # run a single bench
#
# Outputs CSV to benchmarks/history.csv with columns:
#   date,commit,bench_name,time_ns,unit

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CSV="${REPO_ROOT}/benchmarks/history.csv"
mkdir -p "$(dirname "$CSV")"

DATE="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
COMMIT="$(git -C "$REPO_ROOT" rev-parse --short HEAD 2>/dev/null || echo "unknown")"

# Write header if file doesn't exist
if [[ ! -f "$CSV" ]]; then
    echo "date,commit,bench_name,time_ns,unit" > "$CSV"
fi

BENCHES=("compose" "encode" "audio" "convert")
if [[ $# -gt 0 ]]; then
    BENCHES=("$@")
fi

echo "Running benchmarks: ${BENCHES[*]}"
echo "Commit: $COMMIT"
echo ""

for bench in "${BENCHES[@]}"; do
    echo "--- $bench ---"
    # Run criterion and capture output
    OUTPUT=$(cargo bench --bench "$bench" 2>&1) || true
    echo "$OUTPUT" | tail -5

    # Parse criterion output lines like:
    #   bench_name          time:   [1.234 ms 1.256 ms 1.278 ms]
    echo "$OUTPUT" | grep -E '^\S.*time:' | while IFS= read -r line; do
        # Extract benchmark name (first word) and median time (middle value in brackets)
        NAME=$(echo "$line" | awk '{print $1}')
        # Extract the middle value and unit from [low median high]
        MEDIAN=$(echo "$line" | sed -n 's/.*\[\([0-9.]* [a-zµ]*\) \([0-9.]* [a-zµ]*\) .*/\2/p')
        VALUE=$(echo "$MEDIAN" | awk '{print $1}')
        UNIT=$(echo "$MEDIAN" | awk '{print $2}')

        # Convert to nanoseconds for consistent CSV
        case "$UNIT" in
            ns)  NS=$(echo "$VALUE" | awk '{printf "%.0f", $1}') ;;
            µs|"µs")  NS=$(echo "$VALUE" | awk '{printf "%.0f", $1 * 1000}') ;;
            ms)  NS=$(echo "$VALUE" | awk '{printf "%.0f", $1 * 1000000}') ;;
            s)   NS=$(echo "$VALUE" | awk '{printf "%.0f", $1 * 1000000000}') ;;
            *)   NS="$VALUE" ;;
        esac

        if [[ -n "$NS" && "$NS" != "0" ]]; then
            echo "${DATE},${COMMIT},${NAME},${NS},${UNIT}" >> "$CSV"
        fi
    done
    echo ""
done

LINES=$(wc -l < "$CSV")
echo "Done. ${LINES} entries in $CSV"
