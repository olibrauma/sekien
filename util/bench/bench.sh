#!/bin/bash
# sekien benchmark — wall time + max RSS
#
# 使い方:
#   ./bench.sh
#   SEKIEN_BIN=../../target/release/sekien ./bench.sh
#   WARMUP_RUNS=5 BENCH_RUNS=20 ./bench.sh
#
# 出力: Markdown table (stdout)。進捗は stderr。
# mmdc が PATH にあれば sekien と比較する。
#
# 依存: /usr/bin/time (GNU time 1.9+), awk, sort, sed
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
BINARY="${SEKIEN_BIN:-$SCRIPT_DIR/../../target/release/sekien}"
WARMUP_RUNS="${WARMUP_RUNS:-3}"
BENCH_RUNS="${BENCH_RUNS:-10}"
DIAGRAMS_DIR="$SCRIPT_DIR/diagrams"
DIAGRAMS=(flowchart.mmd gitgraph.mmd sequence.mmd)

TIMEFILE=$(mktemp /tmp/sekien_bench_XXXXXX)
TMPSVG=$(mktemp /tmp/sekien_bench_svg_XXXXXX.svg)
trap 'rm -f "$TIMEFILE" "$TMPSVG"' EXIT

[ -f "$BINARY" ] || {
    echo "Error: binary not found: $BINARY" >&2
    echo "hint: cargo build --release" >&2
    exit 1
}

has_mmdc() { command -v mmdc >/dev/null 2>&1; }

# 1 回計測。/usr/bin/time -f "%e %M" -o FILE で elapsed(sec) と maxRSS(kB) を
# ファイルに書き出す。コマンドの stdout/stderr は捨てる。
measure() {
    /usr/bin/time -f "%e %M" -o "$TIMEFILE" "$@" >/dev/null 2>/dev/null
}

# 整数のリストの中央値 (lower median)
median() {
    local n=$#
    printf '%s\n' "$@" | sort -n | sed -n "$(( (n + 1) / 2 ))p"
}

# bench <cmd...> → "median_ms median_rss_kb" を echo
bench() {
    local i elapsed_s rss_kb ms_val
    local times=() rsses=()

    for (( i = 0; i < WARMUP_RUNS; i++ )); do
        "$@" >/dev/null 2>/dev/null
    done
    for (( i = 0; i < BENCH_RUNS; i++ )); do
        measure "$@"
        read -r elapsed_s rss_kb < "$TIMEFILE"
        ms_val=$(awk "BEGIN { printf \"%.0f\", $elapsed_s * 1000 }")
        times+=("$ms_val")
        rsses+=("$rss_kb")
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
    printf "| diagram | sekien time | mmdc time | sekien RSS | mmdc RSS |\n"
    printf "|---|---|---|---|---|\n"
else
    printf "| diagram | time | RSS |\n"
    printf "|---|---|---|\n"
fi

for diag in "${DIAGRAMS[@]}"; do
    diag_path="$DIAGRAMS_DIR/$diag"
    [ -f "$diag_path" ] || { echo "warning: $diag not found, skipping" >&2; continue; }

    printf "  %-20s" "$diag" >&2
    read -r s_ms s_rss < <(bench "$BINARY" "$diag_path")

    if $mmdc_present; then
        read -r m_ms m_rss < <(bench mmdc -p "$puppeteer_cfg" -i "$diag_path" -o "$TMPSVG")
        printf "| %s | %s | %s | %s | %s |\n" \
            "$diag" "$(fmt_ms "$s_ms")" "$(fmt_ms "$m_ms")" \
            "$(fmt_rss "$s_rss")" "$(fmt_rss "$m_rss")"
    else
        printf "| %s | %s | %s |\n" "$diag" "$(fmt_ms "$s_ms")" "$(fmt_rss "$s_rss")"
    fi
    echo >&2
done
