#!/bin/bash
# sekien benchmark — wall time + max RSS (all child processes included)
#
# Usage:
#   ./bench.sh
#   SEKIEN_BIN=../../target/release/sekien ./bench.sh
#   WARMUP_RUNS=5 BENCH_RUNS=20 ./bench.sh
#
# Output: Markdown table on stdout. Progress on stderr.
# If mmdc is on PATH, results are compared against it.
#
# RSS is measured by walking the full PPID chain of the target process,
# capturing Xvfb/WebKit for sekien and Chromium for mmdc.
# Sampled every 10 ms; the median peak value is reported.
#
# Dependencies: ps, awk, sort, sed, date (GNU coreutils)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
BINARY="${SEKIEN_BIN:-$SCRIPT_DIR/../../target/release/sekien}"
WARMUP_RUNS="${WARMUP_RUNS:-3}"
BENCH_RUNS="${BENCH_RUNS:-10}"
DIAGRAMS_DIR="$SCRIPT_DIR/diagrams"
DIAGRAMS=(flowchart.mmd gitgraph.mmd sequence.mmd)

DATAFILE=$(mktemp /tmp/sekien_bench_XXXXXX)
TMPSVG=$(mktemp /tmp/sekien_bench_svg_XXXXXX.svg)
trap 'rm -f "$DATAFILE" "$TMPSVG"' EXIT

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

# Run one measurement. Writes "elapsed_ms max_rss_kb" to $DATAFILE.
measure() {
    local t0 t1 max_rss=0 rss pid
    t0=$(date +%s%3N)
    "$@" >/dev/null 2>/dev/null &
    pid=$!
    while kill -0 "$pid" 2>/dev/null; do
        rss=$(tree_rss_kb "$pid")
        [ "${rss:-0}" -gt "$max_rss" ] && max_rss="${rss:-0}"
        sleep 0.01
    done
    wait "$pid" 2>/dev/null
    t1=$(date +%s%3N)
    printf '%d %d\n' "$(( t1 - t0 ))" "$max_rss" > "$DATAFILE"
}

# Lower median of a list of integers.
median() {
    local n=$#
    printf '%s\n' "$@" | sort -n | sed -n "$(( (n + 1) / 2 ))p"
}

# bench <cmd...> → echoes "median_ms median_rss_kb"
bench() {
    local i ms rss times=() rsses=()
    for (( i = 0; i < WARMUP_RUNS; i++ )); do
        "$@" >/dev/null 2>/dev/null
    done
    for (( i = 0; i < BENCH_RUNS; i++ )); do
        measure "$@"
        read -r ms rss < "$DATAFILE"
        times+=("$ms")
        rsses+=("$rss")
        printf '.' >&2
    done
    echo "$(median "${times[@]}") $(median "${rsses[@]}")"
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
echo "_(warmup ${WARMUP_RUNS} runs, measurement ${BENCH_RUNS} runs, median)_"
echo ""

if $mmdc_present; then
    puppeteer_cfg="$SCRIPT_DIR/puppeteer-config.json"
fi

rows=()
for diag in "${DIAGRAMS[@]}"; do
    diag_path="$DIAGRAMS_DIR/$diag"
    [ -f "$diag_path" ] || { echo "warning: $diag not found, skipping" >&2; continue; }

    printf "  %-20s" "$diag" >&2
    read -r s_ms s_rss < <(bench "$BINARY" "$diag_path")

    if $mmdc_present; then
        read -r m_ms m_rss < <(bench mmdc -p "$puppeteer_cfg" -i "$diag_path" -o "$TMPSVG")
        rows+=("$(printf "| %s | %s | %s | %s | %s |" \
            "$diag" "$(fmt_ms "$s_ms")" "$(fmt_ms "$m_ms")" \
            "$(fmt_rss "$s_rss")" "$(fmt_rss "$m_rss")")")
    else
        rows+=("$(printf "| %s | %s | %s |" "$diag" "$(fmt_ms "$s_ms")" "$(fmt_rss "$s_rss")")")
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
