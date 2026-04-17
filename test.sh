#!/bin/bash
# mmsvg テストスクリプト
# 使い方: ./test.sh
set -euo pipefail

BINARY="./target/debug/mmsvg"

pass() { echo "  PASS: $1"; }
fail() { echo "  FAIL: $1"; FAILED=$((FAILED + 1)); }
FAILED=0

echo "=== mmsvg test ==="
echo ""

# --- 前提確認 ---
echo "[0] 前提確認"
[ -f "$BINARY" ] && pass "binary exists" || { fail "binary not found — run: cargo build"; exit 1; }
echo ""

# --- テスト 1: .mmd ファイル → SVG (stdout) ---
echo "[1] .mmd → SVG (stdout)"
MMD=$(mktemp /tmp/mmsvg_test_XXXXXX.mmd)
printf 'graph LR\n  A --> B\n' > "$MMD"
SVG=$("$BINARY" "$MMD" 2>/dev/null)
rm -f "$MMD"

if echo "$SVG" | grep -q '<svg'; then
  pass ".mmd input produces SVG on stdout"
else
  fail ".mmd input: no SVG in stdout"
fi
if echo "$SVG" | grep -q 'tspan'; then
  pass "SVG contains tspan (text rendered)"
else
  fail "SVG missing tspan"
fi
echo ""

# --- テスト 2: stdin → SVG (stdout) ---
echo "[2] stdin → SVG (stdout)"
RESULT=$(printf 'graph LR\n  A --> B\n' | "$BINARY" 2>/dev/null)
if echo "$RESULT" | grep -q '<svg'; then
  pass "stdin produces SVG on stdout"
else
  fail "stdin: no SVG in stdout"
fi
echo ""

# --- テスト 3: pandoc filter → HTML ---
echo "[3] pandoc filter → HTML"
if ! command -v pandoc &>/dev/null; then
  echo "  SKIP: pandoc not found"
else
  RESULT=$(printf '# test\n\n```mermaid\ngraph LR\n  A --> B\n```\n' \
    | pandoc -f markdown -t html --filter "$BINARY" 2>/dev/null)
  if echo "$RESULT" | grep -q '<svg'; then
    pass "pandoc filter produces SVG in HTML output"
  else
    fail "pandoc filter: no SVG in HTML output"
  fi
fi
echo ""

# --- 結果 ---
if [ "$FAILED" -eq 0 ]; then
  echo "=== ALL PASSED ==="
else
  echo "=== $FAILED FAILED ==="
  exit 1
fi
