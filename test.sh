#!/bin/bash
# mmsvg テストスクリプト
# 使い方: ./test.sh
set -euo pipefail

BINARY="./target/debug/mmsvg"
MD="$HOME/Downloads/01_invention-summary.md"
EXPECTED_PDF="$HOME/Downloads/01_invention-summary.pdf"
OUT_MD="/tmp/mmsvg_test_out.md"
OUT_PDF="/tmp/mmsvg_test_out.pdf"

pass() { echo "  PASS: $1"; }
fail() { echo "  FAIL: $1"; FAILED=$((FAILED + 1)); }
FAILED=0

echo "=== mmsvg test ==="
echo ""

# --- 前提確認 ---
echo "[0] 前提確認"
[ -f "$BINARY" ] && pass "binary exists" || { fail "binary not found — run: cargo build"; exit 1; }
[ -f "$MD" ]     && pass "test md exists" || { fail "test md not found: $MD"; exit 1; }
echo ""

# --- テスト 1: Markdown 変換 ---
echo "[1] Markdown → Markdown (Mermaid → SVG)"
"$BINARY" "$MD" > "$OUT_MD" 2>/dev/null

# Mermaid ブロックが残っていないこと
if grep -q '```mermaid' "$OUT_MD"; then
  fail "mermaid blocks remain in output"
else
  pass "no mermaid blocks in output"
fi

# SVG が期待数 (3) 含まれること
SVG_COUNT=$(grep -c '<svg' "$OUT_MD" || true)
if [ "$SVG_COUNT" -eq 3 ]; then
  pass "SVG count = 3"
else
  fail "SVG count = $SVG_COUNT (expected 3)"
fi

# 日本語テキストが SVG に含まれること
if grep -q 'tspan' "$OUT_MD"; then
  pass "SVG contains tspan (text rendered)"
else
  fail "no tspan in SVG — text may be missing"
fi
echo ""

# --- テスト 2: Pandoc で PDF 化 ---
echo "[2] mmsvg | pandoc → PDF"
if ! command -v pandoc &>/dev/null; then
  echo "  SKIP: pandoc not found"
elif ! command -v typst &>/dev/null; then
  echo "  SKIP: typst not found"
else
  "$BINARY" "$MD" 2>/dev/null \
    | pandoc -f markdown -o "$OUT_PDF" --pdf-engine=typst 2>/dev/null

  if [ -f "$OUT_PDF" ] && [ -s "$OUT_PDF" ]; then
    pass "PDF generated: $OUT_PDF"

    # ファイルサイズを期待 PDF と比較 (10% 以内の差を許容)
    EXPECTED_SIZE=$(wc -c < "$EXPECTED_PDF")
    ACTUAL_SIZE=$(wc -c < "$OUT_PDF")
    RATIO=$(echo "scale=2; $ACTUAL_SIZE * 100 / $EXPECTED_SIZE" | bc)
    if [ "$(echo "$RATIO > 50" | bc)" -eq 1 ] && [ "$(echo "$RATIO < 200" | bc)" -eq 1 ]; then
      pass "PDF size ratio vs expected = ${RATIO}%"
    else
      fail "PDF size ratio vs expected = ${RATIO}% (out of range)"
    fi
  else
    fail "PDF not generated"
  fi
fi
echo ""

# --- テスト 3: stdin パイプ ---
echo "[3] stdin パイプ"
RESULT=$(echo '# test

```mermaid
graph LR
  A --> B
```' | "$BINARY" 2>/dev/null)

if echo "$RESULT" | grep -q '<svg'; then
  pass "stdin pipe produces SVG"
else
  fail "stdin pipe: no SVG in output"
fi
echo ""

# --- 結果 ---
if [ "$FAILED" -eq 0 ]; then
  echo "=== ALL PASSED ==="
else
  echo "=== $FAILED FAILED ==="
  exit 1
fi
