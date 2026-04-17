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
echo "[1] Markdown → Markdown (Mermaid → SVG files)"
"$BINARY" "$MD" > "$OUT_MD" 2>/dev/null

# Mermaid ブロックが残っていないこと
if grep -q '```mermaid' "$OUT_MD"; then
  fail "mermaid blocks remain in output"
else
  pass "no mermaid blocks in output"
fi

# SVG ファイル参照が期待数 (3) 含まれること
IMG_COUNT=$(grep -c '!\[\](' "$OUT_MD" || true)
if [ "$IMG_COUNT" -eq 3 ]; then
  pass "image references count = 3"
else
  fail "image references count = $IMG_COUNT (expected 3)"
fi

# 参照先の SVG ファイルが実在し、tspan を含むこと
# 相対パスの場合は入力 MD の隣にあるはずなので、そのディレクトリを基準に解決
MD_DIR="$(dirname "$MD")"
MISSING=0
BAD_SVG=0
while IFS= read -r line; do
  path="${line#*![](}"
  path="${path%%)*}"
  # 絶対パスでなければ入力 MD のディレクトリを基準に解決
  [[ "$path" != /* ]] && path="$MD_DIR/$path"
  if [ ! -f "$path" ]; then
    MISSING=$((MISSING + 1))
  elif ! grep -q 'tspan' "$path"; then
    BAD_SVG=$((BAD_SVG + 1))
  fi
done < <(grep '!\[\](' "$OUT_MD")

if [ "$MISSING" -eq 0 ]; then
  pass "all SVG files exist"
else
  fail "$MISSING SVG file(s) not found"
fi
if [ "$BAD_SVG" -eq 0 ]; then
  pass "SVG files contain tspan (text rendered)"
else
  fail "$BAD_SVG SVG file(s) missing tspan"
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

    # ファイルサイズを期待 PDF と比較 (50〜200% を許容)
    EXPECTED_SIZE=$(wc -c < "$EXPECTED_PDF")
    ACTUAL_SIZE=$(wc -c < "$OUT_PDF")
    RATIO=$(echo "scale=2; $ACTUAL_SIZE * 100 / $EXPECTED_SIZE" | bc)
    if [ "$(echo "$RATIO > 50" | bc)" -eq 1 ] && [ "$(echo "$RATIO < 200" | bc)" -eq 1 ]; then
      pass "PDF size ratio vs expected = ${RATIO}%"
    else
      fail "PDF size ratio vs expected = ${RATIO}% (out of range)"
    fi

    # SVG のテキストノードが PDF に漏れていないこと
    # Typst が <svg> を描画できない場合、SVG 内の <tspan> テキストが
    # 連結されてプレーンテキストとして現れる (例: "intidPK", "stringnamePK" など)
    if ! command -v pdftotext &>/dev/null; then
      echo "  SKIP: pdftotext not found (install poppler)"
    else
      PDF_TEXT=$(pdftotext "$OUT_PDF" - 2>/dev/null)
      if echo "$PDF_TEXT" | grep -q 'intidPK'; then
        fail "SVG tspan text leaked into PDF — SVG not rendered by Typst"
      else
        pass "no SVG tspan leak in PDF"
      fi
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

if echo "$RESULT" | grep -q '!\[\](.*\.svg)'; then
  pass "stdin pipe produces SVG file reference"
else
  fail "stdin pipe: no SVG file reference in output"
fi
echo ""

# --- 結果 ---
if [ "$FAILED" -eq 0 ]; then
  echo "=== ALL PASSED ==="
else
  echo "=== $FAILED FAILED ==="
  exit 1
fi
