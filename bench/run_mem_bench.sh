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
export GDK_BACKEND=headless

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
    python3 "$MEM_RSS_PY" "$SEKIEN" "$f"
    
    echo "[mmdc]"
    python3 "$MEM_RSS_PY" mmdc -p "$PUPPETEER_CONFIG" -i "$f" -o "${f}.svg"
    
    echo ""
done

echo "Benchmark complete."
