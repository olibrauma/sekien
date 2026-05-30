#!/bin/bash
# sekien E2E テストスクリプト
# protocol.md (v1.0) の仕様に基づく入出力検証。
#
# 使い方:
#   ./test.sh                   # <repo>/target/debug/sekien を使用
#   SEKIEN_BIN=path/to/sekien ./test.sh
#
# 終了コード: 0=全 PASS, 1=1件以上 FAIL
#
# 注意: NUL バイトを含む入出力は bash の $() コマンド置換で消えるため、
#       NUL を扱うテストはすべてテンポラリファイル経由で処理する。
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
BINARY="${SEKIEN_BIN:-$REPO_ROOT/target/debug/sekien}"
FAILED=0
TMPOUT=$(mktemp /tmp/sekien_test_out_XXXXXX)
TMPERR=$(mktemp /tmp/sekien_test_err_XXXXXX)

cleanup() { rm -f "$TMPOUT" "$TMPERR"; }
trap cleanup EXIT

pass() { echo "  PASS: $1"; }
fail() { echo "  FAIL: $1"; FAILED=$((FAILED + 1)); }

# ファイル中の <svg 出現回数
count_svg_in_file() { grep -c '<svg' "$1" 2>/dev/null || echo 0; }

# ファイル中の NUL バイト数
count_nul_in_file() { tr -cd '\0' < "$1" | wc -c; }

echo "=== sekien E2E test ==="
echo ""

# --- 前提確認 ---
echo "[0] 前提確認"
[ -f "$BINARY" ] && pass "binary exists ($BINARY)" || {
    fail "binary not found — run: cargo build"
    exit 1
}
echo ""

# --- テスト 1: ファイル引数 → SVG (stdout) ---
echo "[1] ファイル引数 → SVG (stdout)"
MMD=$(mktemp /tmp/sekien_test_XXXXXX.mmd)
printf 'graph LR\n  A --> B\n' > "$MMD"
"$BINARY" "$MMD" > "$TMPOUT" 2>"$TMPERR"
EXIT=$?
rm -f "$MMD"

[ $EXIT -eq 0 ] && pass "exit 0" || fail "exit $EXIT (expected 0)"
SVG_COUNT=$(count_svg_in_file "$TMPOUT")
[ "$SVG_COUNT" -gt 0 ] && pass "stdout contains <svg" || fail "stdout missing <svg"
grep -qF 'tspan' "$TMPOUT" && pass "SVG contains tspan (text rendered)" || fail "SVG missing tspan"
echo ""

# --- テスト 2: stdin 単一ブロック → SVG, NUL なし ---
echo "[2] stdin 単一ブロック → SVG (NUL なし)"
printf 'graph LR\n  A --> B\n' | "$BINARY" > "$TMPOUT" 2>"$TMPERR"
EXIT=$?
[ $EXIT -eq 0 ] && pass "exit 0" || fail "exit $EXIT (expected 0)"
SVG_COUNT=$(count_svg_in_file "$TMPOUT")
[ "$SVG_COUNT" -gt 0 ] && pass "stdout contains <svg" || fail "stdout missing <svg"
NUL_COUNT=$(count_nul_in_file "$TMPOUT")
[ "$NUL_COUNT" -eq 0 ] && pass "stdout no NUL for single block" || fail "stdout has $NUL_COUNT NUL(s) (expected 0)"
echo ""

# --- テスト 3: stdin マルチブロック (NUL 区切り) → 2 SVG + NUL 1個 ---
echo "[3] stdin マルチブロック → 2 SVG, NUL 区切り 1 個"
printf 'graph LR\n  A --> B\x00graph TD\n  X --> Y' | "$BINARY" > "$TMPOUT" 2>"$TMPERR"
EXIT=$?
[ $EXIT -eq 0 ] && pass "exit 0" || fail "exit $EXIT (expected 0)"
SVG_COUNT=$(count_svg_in_file "$TMPOUT")
[ "$SVG_COUNT" -eq 2 ] && pass "stdout contains 2 SVGs" || fail "stdout has $SVG_COUNT SVG(s) (expected 2)"
NUL_COUNT=$(count_nul_in_file "$TMPOUT")
[ "$NUL_COUNT" -eq 1 ] && pass "stdout has 1 NUL separator" || fail "stdout has $NUL_COUNT NUL(s) (expected 1)"
echo ""

# --- テスト 4: マルチブロック出力順序の保証 ---
echo "[4] マルチブロック出力順序"
printf 'graph LR\n  Alpha --> Beta\x00graph LR\n  Gamma --> Delta' | "$BINARY" > "$TMPOUT" 2>"$TMPERR"
POS_ALPHA=$(grep -abo 'Alpha' "$TMPOUT" | head -1 | cut -d: -f1 || echo -1)
POS_GAMMA=$(grep -abo 'Gamma' "$TMPOUT" | head -1 | cut -d: -f1 || echo -1)
[ "${POS_ALPHA:--1}" -ge 0 ] && [ "${POS_GAMMA:--1}" -ge 0 ] || {
    fail "Alpha または Gamma がstdout に見つからない (Alpha@$POS_ALPHA, Gamma@$POS_GAMMA)"
    echo ""
}
[ "${POS_ALPHA:--1}" -lt "${POS_GAMMA:--1}" ] \
    && pass "出力順序: Alpha(block1) < Gamma(block2)" \
    || fail "出力順序が逆転: Alpha@$POS_ALPHA, Gamma@$POS_GAMMA"
echo ""

# --- テスト 5: 末尾 NUL は無視される ---
echo "[5] 末尾 NUL の無視"
printf 'graph LR\n  A --> B\x00' | "$BINARY" > "$TMPOUT" 2>"$TMPERR"
EXIT=$?
[ $EXIT -eq 0 ] && pass "exit 0" || fail "exit $EXIT (expected 0)"
SVG_COUNT=$(count_svg_in_file "$TMPOUT")
[ "$SVG_COUNT" -eq 1 ] && pass "stdout contains 1 SVG (trailing NUL ignored)" || fail "stdout has $SVG_COUNT SVG(s) (expected 1)"
[ ! -s "$TMPERR" ] && pass "stderr empty" || fail "stderr not empty: $(cat "$TMPERR")"
echo ""

# --- テスト 6: エラー継続 (continue-on-error) ---
echo "[6] エラー継続: 3 ブロック中 2 番目が不正"
printf 'graph LR\n  A --> B\x00thisIsNotValidMermaid\x00graph TD\n  X --> Y' \
    | "$BINARY" --block-id > "$TMPOUT" 2>"$TMPERR"
EXIT=$?
[ $EXIT -eq 0 ] && pass "exit 0 (continue-on-error)" || fail "exit $EXIT (expected 0)"
SVG_COUNT=$(count_svg_in_file "$TMPOUT")
[ "$SVG_COUNT" -eq 2 ] && pass "stdout contains 2 SVGs (valid blocks only)" || fail "stdout has $SVG_COUNT SVG(s) (expected 2)"
NUL_COUNT=$(count_nul_in_file "$TMPOUT")
[ "$NUL_COUNT" -eq 1 ] && pass "stdout has 1 NUL separator" || fail "stdout has $NUL_COUNT NUL(s) (expected 1)"
grep -qF '<!-- {"id": 2} -->' "$TMPERR" \
    && pass "stderr contains block-id comment for failed block 2" \
    || fail "stderr missing <!-- {\"id\": 2} -->: $(cat "$TMPERR")"
grep -qF 'No diagram type detected' "$TMPERR" \
    && pass "stderr contains mermaid error text" \
    || fail "stderr missing mermaid error text: $(cat "$TMPERR")"
echo ""

# --- テスト 7: 全ブロック失敗 → stdout 空, stderr に全ブロック分のエラー ---
echo "[7] 全ブロック失敗 → stdout 空, stderr に 3 ブロック分のエラー"
printf 'bogusOne\x00bogusTwo\x00bogusThree' \
    | "$BINARY" --block-id > "$TMPOUT" 2>"$TMPERR"
EXIT=$?
[ $EXIT -eq 0 ] && pass "exit 0" || fail "exit $EXIT (expected 0)"
[ ! -s "$TMPOUT" ] && pass "stdout empty" || fail "stdout not empty ($(wc -c < "$TMPOUT") bytes)"
COUNT_IDS=$(grep -c '<!-- {"id":' "$TMPERR" || true)
[ "$COUNT_IDS" -eq 3 ] && pass "stderr has 3 block-id error lines" || fail "stderr has $COUNT_IDS block-id lines (expected 3)"
echo ""

# --- テスト 8: --block-id → stdout に JSON コメントが付く ---
echo "[8] --block-id オプション → stdout に <!-- {\"id\": N} --> が付く"
printf 'graph LR\n  A --> B\n' | "$BINARY" --block-id > "$TMPOUT" 2>"$TMPERR"
EXIT=$?
[ $EXIT -eq 0 ] && pass "exit 0" || fail "exit $EXIT (expected 0)"
grep -qF '<!-- {"id": 1} -->' "$TMPOUT" \
    && pass 'stdout contains <!-- {"id": 1} -->' \
    || fail "stdout missing <!-- {\"id\": 1} -->: $(head -3 "$TMPOUT")"
echo ""

# --- テスト 9: --theme オプション ---
echo "[9] --theme dark → SVG を生成"
MMD=$(mktemp /tmp/sekien_test_XXXXXX.mmd)
printf 'graph LR\n  A --> B\n' > "$MMD"
"$BINARY" --theme dark "$MMD" > "$TMPOUT" 2>"$TMPERR"
EXIT=$?
rm -f "$MMD"
[ $EXIT -eq 0 ] && pass "exit 0" || fail "exit $EXIT (expected 0)"
SVG_COUNT=$(count_svg_in_file "$TMPOUT")
[ "$SVG_COUNT" -gt 0 ] && pass "stdout contains <svg" || fail "stdout missing <svg"
echo ""

# --- テスト 10: --font オプション ---
echo "[10] --font Arial → SVG を生成"
MMD=$(mktemp /tmp/sekien_test_XXXXXX.mmd)
printf 'graph LR\n  A --> B\n' > "$MMD"
"$BINARY" --font Arial "$MMD" > "$TMPOUT" 2>"$TMPERR"
EXIT=$?
rm -f "$MMD"
[ $EXIT -eq 0 ] && pass "exit 0" || fail "exit $EXIT (expected 0)"
SVG_COUNT=$(count_svg_in_file "$TMPOUT")
[ "$SVG_COUNT" -gt 0 ] && pass "stdout contains <svg" || fail "stdout missing <svg"
echo ""

# --- テスト 11: --look オプション ---
echo "[11] --look classic → SVG を生成"
MMD=$(mktemp /tmp/sekien_test_XXXXXX.mmd)
printf 'graph LR\n  A --> B\n' > "$MMD"
"$BINARY" --look classic "$MMD" > "$TMPOUT" 2>"$TMPERR"
EXIT=$?
rm -f "$MMD"
[ $EXIT -eq 0 ] && pass "exit 0" || fail "exit $EXIT (expected 0)"
SVG_COUNT=$(count_svg_in_file "$TMPOUT")
[ "$SVG_COUNT" -gt 0 ] && pass "stdout contains <svg" || fail "stdout missing <svg"
echo ""

# --- テスト 12: --config オプション ---
echo "[12] --config → SVG を生成"
CFGTMP=$(mktemp /tmp/sekien_test_XXXXXX.json)
printf '{"theme":"dark"}' > "$CFGTMP"
printf 'graph LR\n  A --> B\n' | "$BINARY" --config "$CFGTMP" > "$TMPOUT" 2>"$TMPERR"
EXIT=$?
rm -f "$CFGTMP"
[ $EXIT -eq 0 ] && pass "exit 0" || fail "exit $EXIT (expected 0)"
SVG_COUNT=$(count_svg_in_file "$TMPOUT")
[ "$SVG_COUNT" -gt 0 ] && pass "stdout contains <svg" || fail "stdout missing <svg"
echo ""

# --- テスト 13: ファイル複数指定 → exit 非 0 ---
echo "[13] ファイル複数指定 → exit 非 0"
MMD1=$(mktemp /tmp/sekien_test_XXXXXX.mmd)
MMD2=$(mktemp /tmp/sekien_test_XXXXXX.mmd)
printf 'graph LR\n  A --> B\n' > "$MMD1"
printf 'graph TD\n  X --> Y\n' > "$MMD2"
"$BINARY" "$MMD1" "$MMD2" > "$TMPOUT" 2>"$TMPERR"
EXIT=$?
rm -f "$MMD1" "$MMD2"
[ $EXIT -ne 0 ] && pass "exit non-0 ($EXIT)" || fail "exit 0 (expected non-0)"
echo ""

# --- テスト 14: --version ---
echo "[14] --version → 'sekien' と 'mermaid' を含む"
"$BINARY" --version > "$TMPOUT" 2>"$TMPERR"
EXIT=$?
[ $EXIT -eq 0 ] && pass "exit 0" || fail "exit $EXIT (expected 0)"
grep -qF 'sekien'  "$TMPOUT" && pass "stdout contains 'sekien'"  || fail "stdout missing 'sekien'"
grep -qF 'mermaid' "$TMPOUT" && pass "stdout contains 'mermaid'" || fail "stdout missing 'mermaid'"
echo ""

# --- テスト 15: --help ---
echo "[15] --help → exit 0"
"$BINARY" --help > "$TMPOUT" 2>"$TMPERR"
EXIT=$?
[ $EXIT -eq 0 ] && pass "exit 0" || fail "exit $EXIT (expected 0)"
echo ""

# --- テスト 16: NUL 直後の whitespace は空ブロックにならず無視される ---
echo "[16] NUL 直後の whitespace → 無視"
printf 'graph LR\n  A --> B\x00\n' | "$BINARY" > "$TMPOUT" 2>"$TMPERR"
EXIT=$?
[ $EXIT -eq 0 ] && pass "exit 0" || fail "exit $EXIT (expected 0)"
SVG_COUNT=$(count_svg_in_file "$TMPOUT")
[ "$SVG_COUNT" -eq 1 ] && pass "stdout contains 1 SVG" || fail "stdout has $SVG_COUNT SVG(s) (expected 1)"
[ ! -s "$TMPERR" ] && pass "stderr empty" || fail "stderr not empty: $(cat "$TMPERR")"
echo ""

# --- 結果 ---
if [ "$FAILED" -eq 0 ]; then
    echo "=== ALL PASSED ==="
else
    echo "=== $FAILED FAILED ==="
    exit 1
fi
