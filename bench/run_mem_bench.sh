#!/bin/bash
# sekien vs mmdc メモリ使用量比較スクリプト
# 使い方: ./bench/run_mem_bench.sh
set -eu

BASE_DIR=$(cd "$(dirname "$0")/.." && pwd)
SEKIEN="${BASE_DIR}/target/release/sekien"
PUPPETEER_CONFIG="${BASE_DIR}/bench/puppeteer-config.json"
MEM_RSS_PY="${BASE_DIR}/bench/mem_rss.py"
DIAGRAMS_DIR="${BASE_DIR}/bench/diagrams"
FILES="flowchart.mmd gitgraph.mmd sequence.mmd"

# バイナリの存在確認
if [ ! -f "$SEKIEN" ]; then
    echo "Error: $SEKIEN not found. Please run 'cargo build --release' first."
    exit 1
fi

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

echo "=== sekien Memory Footprint Benchmark ==="
echo ""

# テンポラリディレクトリで実行
OUT_DIR=$(mktemp -d /tmp/sekien-mem-XXXXXX)
trap 'rm -rf "$OUT_DIR"' EXIT
cp "${DIAGRAMS_DIR}"/*.mmd "$OUT_DIR/"
cd "$OUT_DIR"

for f in $FILES; do
    echo "--- Benchmarking $f ---"
    
    echo "[sekien]"
    python3 "$MEM_RSS_PY" ${CMD_WRAPPER} "$SEKIEN" "$f"
    
    echo "[mmdc]"
    python3 "$MEM_RSS_PY" ${CMD_WRAPPER} mmdc -p "$PUPPETEER_CONFIG" -i "$f" -o "${f}.svg"
    
    echo ""
done

echo "Benchmark complete."
