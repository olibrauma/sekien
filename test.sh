#!/bin/bash
# sekien 統合テストスクリプト
# 使い方: ./test.sh
set -uo pipefail

SEKIEN="./target/debug/sekien"
SEKIEN_PANDOC="./target/debug/sekien-pandoc"

# Linux ヘッドレス環境 (xvfb-run 等) 向けの設定
if [[ "$OSTYPE" == "linux-gnu"* ]]; then
  export GDK_BACKEND=x11
  export WEBKIT_DISABLE_COMPOSITING_MODE=1
fi

pass() { echo "  PASS: $1"; }
fail() { echo "  FAIL: $1"; FAILED=$((FAILED + 1)); }
skip() { echo "  SKIP: $1"; }
FAILED=0

echo "=== sekien integration test ==="
echo ""

# --- [0] 前提確認 ---
echo "[0] 前提確認"
[ -f "$SEKIEN" ]        && pass "sekien binary exists"        || { fail "sekien not found — run: cargo build"; exit 1; }
[ -f "$SEKIEN_PANDOC" ] && pass "sekien-pandoc binary exists" || { fail "sekien-pandoc not found — run: cargo build"; exit 1; }
echo ""

# --- ヘルパー ---
mmd_file() {
  local f; f=$(mktemp /tmp/sekien_test_XXXXXX)
  printf 'graph LR\n  A --> B\n' > "$f"
  echo "$f"
}
is_svg() { echo "$1" | grep -q '<svg'; }

# ============================================================
# sekien (スタンドアロン CLI)
# ============================================================

echo "--- sekien ---"
echo ""

# [1] .mmd ファイル → SVG
echo "[1] .mmd ファイル → SVG (stdout)"
MMD=$(mmd_file)
SVG=$("$SEKIEN" "$MMD" 2>/dev/null); rm -f "$MMD"
is_svg "$SVG"                          && pass ".mmd input produces SVG"      || fail ".mmd input: no SVG"
echo "$SVG" | grep -q 'tspan'         && pass "SVG contains tspan"            || fail "SVG missing tspan"
echo ""

# [2] stdin → SVG
echo "[2] stdin → SVG (stdout)"
SVG=$(printf 'graph LR\n  A --> B\n' | "$SEKIEN" 2>/dev/null)
is_svg "$SVG" && pass "stdin produces SVG" || fail "stdin: no SVG"
echo ""

# [3] --version
echo "[3] --version"
OUT=$("$SEKIEN" --version 2>/dev/null)
echo "$OUT" | grep -q 'sekien'    && pass "--version output contains 'sekien'"    || fail "--version: unexpected output"
echo "$OUT" | grep -q 'mermaid'   && pass "--version output contains 'mermaid'"   || fail "--version: no mermaid version"
echo ""

# [4] --help
echo "[4] --help"
"$SEKIEN" --help >/dev/null 2>&1 && pass "--help exits 0" || fail "--help exited non-zero"
echo ""

# [5] --font フラグ
echo "[5] --font フラグ"
MMD=$(mmd_file)
SVG=$("$SEKIEN" --font "Arial" "$MMD" 2>/dev/null); rm -f "$MMD"
is_svg "$SVG" && pass "--font: produces SVG" || fail "--font: no SVG"
echo ""

# [6] --theme フラグ
echo "[6] --theme フラグ"
MMD=$(mmd_file)
SVG=$("$SEKIEN" --theme dark "$MMD" 2>/dev/null); rm -f "$MMD"
is_svg "$SVG" && pass "--theme: produces SVG" || fail "--theme: no SVG"
echo ""

# [7] --look フラグ
echo "[7] --look フラグ"
MMD=$(mmd_file)
SVG=$("$SEKIEN" --look classic "$MMD" 2>/dev/null); rm -f "$MMD"
is_svg "$SVG" && pass "--look: produces SVG" || fail "--look: no SVG"
echo ""

# [8] SEKIEN_FONT env var
echo "[8] SEKIEN_FONT env var"
MMD=$(mmd_file)
SVG=$(SEKIEN_FONT="Arial" "$SEKIEN" "$MMD" 2>/dev/null); rm -f "$MMD"
is_svg "$SVG" && pass "SEKIEN_FONT: produces SVG" || fail "SEKIEN_FONT: no SVG"
echo ""

# [9] ファイルが多すぎる場合はエラー
echo "[9] 複数ファイル指定はエラー"
MMD1=$(mmd_file); MMD2=$(mmd_file)
"$SEKIEN" "$MMD1" "$MMD2" >/dev/null 2>&1 && fail "multiple files: should exit non-zero" || pass "multiple files: exits non-zero"
rm -f "$MMD1" "$MMD2"
echo ""

# ============================================================
# sekien-pandoc (Pandoc フィルタ)
# ============================================================

echo "--- sekien-pandoc ---"
echo ""

# [10] --version
echo "[10] --version"
OUT=$("$SEKIEN_PANDOC" --version 2>/dev/null)
echo "$OUT" | grep -q 'sekien-pandoc' && pass "--version output contains 'sekien-pandoc'" || fail "--version: unexpected output"
echo "$OUT" | grep -q 'mermaid'       && pass "--version output contains 'mermaid'"       || fail "--version: no mermaid version"
echo ""

# [11] --help
echo "[11] --help"
"$SEKIEN_PANDOC" --help >/dev/null 2>&1 && pass "--help exits 0" || fail "--help exited non-zero"
echo ""

# [12] --print-lua-filter
echo "[12] --print-lua-filter"
OUT=$("$SEKIEN_PANDOC" --print-lua-filter 2>/dev/null)
echo "$OUT" | grep -q 'function'  && pass "--print-lua-filter outputs Lua" || fail "--print-lua-filter: no Lua content"
echo ""

# pandoc 依存テストはここから
if ! command -v pandoc &>/dev/null; then
  skip "[13]-[16]: pandoc not found"
  echo ""
else

# [13] pandoc filter: Mermaid ブロック → SVG in HTML
echo "[13] pandoc filter: Mermaid ブロック → SVG"
RESULT=$(printf '# test\n\n```mermaid\ngraph LR\n  A --> B\n```\n' \
  | pandoc -f markdown -t html --filter "$SEKIEN_PANDOC" 2>/dev/null)
is_svg "$RESULT" && pass "pandoc filter produces SVG in HTML" || fail "pandoc filter: no SVG"
echo ""

# [14] pandoc filter: Mermaid ブロックなし → パススルー
echo "[14] pandoc filter: Mermaid なし → パススルー"
RESULT=$(printf '# hello\n\nsome text\n' \
  | pandoc -f markdown -t html --filter "$SEKIEN_PANDOC" 2>/dev/null)
echo "$RESULT" | grep -q 'hello'   && pass "non-mermaid content preserved"  || fail "non-mermaid: content missing"
is_svg "$RESULT"                   && fail "non-mermaid: unexpected SVG"     || pass "non-mermaid: no SVG (correct)"
echo ""

# [15] pandoc filter: 複数の Mermaid ブロック
echo "[15] pandoc filter: 複数の Mermaid ブロック"
RESULT=$(printf '```mermaid\ngraph LR\n  A --> B\n```\n\n```mermaid\ngraph TD\n  X --> Y\n```\n' \
  | pandoc -f markdown -t html --filter "$SEKIEN_PANDOC" 2>/dev/null)
SVG_COUNT=$(echo "$RESULT" | grep -c '<svg')
[ "$SVG_COUNT" -ge 2 ] && pass "multiple mermaid blocks: $SVG_COUNT SVGs produced" || fail "multiple blocks: expected >=2 SVGs, got $SVG_COUNT"
echo ""

# [16] pandoc filter: SEKIEN_FONT env var
echo "[16] pandoc filter: SEKIEN_FONT env var"
RESULT=$(printf '```mermaid\ngraph LR\n  A --> B\n```\n' \
  | SEKIEN_FONT="Arial" pandoc -f markdown -t html --filter "$SEKIEN_PANDOC" 2>/dev/null)
is_svg "$RESULT" && pass "SEKIEN_FONT: pandoc filter produces SVG" || fail "SEKIEN_FONT: no SVG"
echo ""

fi # pandoc

# --- 結果 ---
if [ "$FAILED" -eq 0 ]; then
  echo "=== ALL PASSED ==="
else
  echo "=== $FAILED FAILED ==="
  exit 1
fi
