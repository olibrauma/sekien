#!/bin/bash
# sekien benchmark — wall time + max RSS (all child processes included)
#
# Usage:
#   ./bench.sh
#   SEKIEN_BIN=../../target/release/sekien ./bench.sh
#   WARMUP_RUNS=5 BENCH_RUNS=20 RSS_RUNS=5 PAUSE=2 ./bench.sh
#
# Output: Markdown table on stdout. Progress on stderr.
# If mmdc is on PATH, results are compared against it.
#
# Time and RSS are measured in separate runs: sampling RSS (ps every 10 ms)
# costs enough CPU to slow the measured process down noticeably.
# RSS is measured by walking the full PPID chain of the target process,
# capturing Xvfb/WebKit for sekien and Chromium for mmdc.
# Sampled every 10 ms; the median peak value is reported.
#
# sekien and mmdc runs alternate, and which one goes first swaps every run:
# on a desktop, background load drifts during a benchmark and would otherwise
# favour whichever tool ran later (or first). Each run is followed by a pause
# (PAUSE seconds, default 1) so that mmdc's Chromium, which keeps tearing down
# after mmdc exits, does not overlap the next run. Still, run it on an idle
# machine; see "Measuring" in util/docs/DESIGN.md.
#
# Dependencies: ps, awk, sort, sed, date (GNU coreutils)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
BINARY="${SEKIEN_BIN:-$SCRIPT_DIR/../../target/release/sekien}"
WARMUP_RUNS="${WARMUP_RUNS:-3}"
BENCH_RUNS="${BENCH_RUNS:-10}"
RSS_RUNS="${RSS_RUNS:-5}"
PAUSE="${PAUSE:-1}"
DIAGRAMS_DIR="$SCRIPT_DIR/diagrams"
DIAGRAMS=(flowchart.mmd gitgraph.mmd sequence.mmd)

TMPSVG=$(mktemp /tmp/sekien_bench_svg_XXXXXX.svg)
trap 'rm -f "$TMPSVG"' EXIT

[ -f "$BINARY" ] || {
    echo "Error: binary not found: $BINARY" >&2
    echo "hint: cargo build --release" >&2
    exit 1
}

has_mmdc() { command -v mmdc >/dev/null 2>&1; }

# Sum RSS of a process and all its descendants by walking the PPID chain (KB).
tree_rss_kb() {
    local root=$1
    ps -e -o pid= -o ppid= -o rss= | awk -v root="$root" '
    { ppid[$1]=$2; rss[$1]=$3; children[$2]=children[$2]" "$1 }
    END {
        q[0]=root; n=1; total=0
        while (n > 0) {
            p = q[--n]
            if (p == "") continue
            total += rss[p]
            split(children[p], c, " ")
            for (i in c) if (c[i] != "") q[n++] = c[i]
        }
        print total
    }'
}

# Wall time of one run, in ms (no sampling, so nothing competes for the CPU).
measure_time() {
    local t0 t1
    t0=$(date +%s%3N)
    "$@" >/dev/null 2>/dev/null
    t1=$(date +%s%3N)
    echo "$(( t1 - t0 ))"
}

# Peak RSS of one run, in KB (the process and all its descendants).
measure_rss() {
    local max_rss=0 rss pid
    "$@" >/dev/null 2>/dev/null &
    pid=$!
    while kill -0 "$pid" 2>/dev/null; do
        rss=$(tree_rss_kb "$pid")
        [ "${rss:-0}" -gt "$max_rss" ] && max_rss="${rss:-0}"
        sleep 0.01
    done
    wait "$pid" 2>/dev/null
    echo "$max_rss"
}

# Lower median of a list of integers.
median() {
    local n=$#
    printf '%s\n' "$@" | sort -n | sed -n "$(( (n + 1) / 2 ))p"
}

# bench_pair <diagram>: runs sekien (and mmdc, if present) alternately, swapping
# the order every run. Sets S_MS S_RSS (and M_MS M_RSS) to the medians.
bench_pair() {
    local diag=$1 st=() sr=() mt=() mr=()
    local sekien=("$BINARY" "$diag")
    local mmdc_cmd=(mmdc -p "$puppeteer_cfg" -i "$diag" -o "$TMPSVG")
    for (( i = 0; i < WARMUP_RUNS; i++ )); do
        "${sekien[@]}" >/dev/null 2>/dev/null
        $mmdc_present && "${mmdc_cmd[@]}" >/dev/null 2>/dev/null
    done
    # Alternate the two tools, swapping which goes first every run.
    pair() {
        local i n=$1 a=$2 b=$3
        for (( i = 0; i < n; i++ )); do
            if ! $mmdc_present; then
                $a
            elif (( i % 2 == 0 )); then
                $a; $b
            else
                $b; $a
            fi
            printf '.' >&2
        done
    }
    time_sekien() { st+=("$(measure_time "${sekien[@]}")"); sleep "$PAUSE"; }
    time_mmdc() { mt+=("$(measure_time "${mmdc_cmd[@]}")"); sleep "$PAUSE"; }
    rss_sekien() { sr+=("$(measure_rss "${sekien[@]}")"); sleep "$PAUSE"; }
    rss_mmdc() { mr+=("$(measure_rss "${mmdc_cmd[@]}")"); sleep "$PAUSE"; }
    pair "$BENCH_RUNS" time_sekien time_mmdc
    pair "$RSS_RUNS" rss_sekien rss_mmdc
    S_MS=$(median "${st[@]}"); S_RSS=$(median "${sr[@]}")
    if $mmdc_present; then M_MS=$(median "${mt[@]}"); M_RSS=$(median "${mr[@]}"); fi
}

fmt_ms() {
    local ms=$1
    if (( ms >= 1000 )); then
        awk "BEGIN { printf \"%.2f s\", $ms / 1000 }"
    else
        echo "${ms} ms"
    fi
}

fmt_rss() {
    awk "BEGIN { printf \"%.0f MB\", $1 / 1024 }"
}

mmdc_present=false
has_mmdc && mmdc_present=true
$mmdc_present || echo "note: mmdc not in PATH — sekien only." >&2

echo "# sekien benchmark results"
echo "_(warmup ${WARMUP_RUNS} runs; median of ${BENCH_RUNS} timed runs and ${RSS_RUNS} RSS runs)_"
echo ""

puppeteer_cfg="$SCRIPT_DIR/puppeteer-config.json"

rows=()
for diag in "${DIAGRAMS[@]}"; do
    diag_path="$DIAGRAMS_DIR/$diag"
    [ -f "$diag_path" ] || { echo "warning: $diag not found, skipping" >&2; continue; }

    printf "  %-20s" "$diag" >&2
    bench_pair "$diag_path"

    if $mmdc_present; then
        rows+=("$(printf "| %s | %s | %s | %s | %s |" \
            "$diag" "$(fmt_ms "$S_MS")" "$(fmt_ms "$M_MS")" \
            "$(fmt_rss "$S_RSS")" "$(fmt_rss "$M_RSS")")")
    else
        rows+=("$(printf "| %s | %s | %s |" "$diag" "$(fmt_ms "$S_MS")" "$(fmt_rss "$S_RSS")")")
    fi
    echo >&2
done

if $mmdc_present; then
    printf "| diagram | sekien time | mmdc time | sekien RSS | mmdc RSS |\n"
    printf "|---|---|---|---|---|\n"
else
    printf "| diagram | time | RSS |\n"
    printf "|---|---|---|\n"
fi
printf "%s\n" "${rows[@]}"
