#!/bin/bash
# sekien vs mmdc 高精度ベンチマークスクリプト
# 使い方: xvfb-run ./run.sh [出力先ディレクトリ]
set -eu

# バイナリとアセットの絶対パスを取得
BASE_DIR=$(pwd)
SEKIEN="${BASE_DIR}/target/release/sekien"
PUPPETEER_CONFIG="${BASE_DIR}/bench/puppeteer-config.json"
DIAGRAMS_DIR="${BASE_DIR}/bench/diagrams"
FILES="flowchart.mmd gitgraph.mmd sequence.mmd"

# 出力先ディレクトリの準備
OUT_DIR=${1:-$(mktemp -d /tmp/sekien-bench-XXXXXX)}
mkdir -p "$OUT_DIR"
cp "${DIAGRAMS_DIR}"/*.mmd "$OUT_DIR/"
cd "$OUT_DIR"

echo "=== sekien Performance Benchmark ==="
echo "Target: ${OUT_DIR}"
echo ""

# Linux ヘッドレス環境向け設定
export GDK_BACKEND=x11
export WEBKIT_DISABLE_COMPOSITING_MODE=1

# xvfb-run の自動判定
CMD_WRAPPER=""
if [[ "$OSTYPE" == "linux-gnu"* ]] && [ -z "${DISPLAY:-}" ]; then
  if command -v xvfb-run >/dev/null 2>&1; then
    CMD_WRAPPER="xvfb-run -a"
  else
    echo "Warning: DISPLAY is not set and xvfb-run is not found. Benchmarks may fail."
  fi
fi

for f in $FILES; do
  echo "--- Benchmarking $f ---"
  hyperfine --warmup 3 --export-markdown "result_${f}.md" \
    "${CMD_WRAPPER} ${SEKIEN} ${f}" \
    "${CMD_WRAPPER} mmdc -p ${PUPPETEER_CONFIG} -i ${f} -o ${f}.svg"
  echo ""
done

echo "=== Results Summary ==="
cat result_*.md
echo ""
echo "Benchmark artifacts are in: ${OUT_DIR}"
