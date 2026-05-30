#!/bin/bash
# sekien end-to-end test script
# Validates stdin/stdout/stderr behaviour against the protocol.md (v1.0) spec.
#
# Usage:
#   ./test.sh                        # uses <repo>/target/debug/sekien
#   SEKIEN_BIN=path/to/sekien ./test.sh
#
# Exit code: 0 = all PASS, 1 = at least one FAIL
#
# Note: NUL bytes are silently dropped by bash $() command substitution,
#       so all tests involving NUL use temporary files instead.
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

# Count occurrences of <svg in a file
count_svg_in_file() { grep -c '<svg' "$1" 2>/dev/null || echo 0; }

# Count NUL bytes in a file
count_nul_in_file() { tr -cd '\0' < "$1" | wc -c; }

echo "=== sekien E2E test ==="
echo ""

# --- Prerequisites ---
echo "[0] Prerequisites"
[ -f "$BINARY" ] && pass "binary exists ($BINARY)" || {
    fail "binary not found — run: cargo build"
    exit 1
}
echo ""

# --- Test 1: file argument → SVG on stdout ---
echo "[1] File argument → SVG on stdout"
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

# --- Test 2: stdin single block → SVG, no NUL ---
echo "[2] stdin single block → SVG (no NUL)"
printf 'graph LR\n  A --> B\n' | "$BINARY" > "$TMPOUT" 2>"$TMPERR"
EXIT=$?
[ $EXIT -eq 0 ] && pass "exit 0" || fail "exit $EXIT (expected 0)"
SVG_COUNT=$(count_svg_in_file "$TMPOUT")
[ "$SVG_COUNT" -gt 0 ] && pass "stdout contains <svg" || fail "stdout missing <svg"
NUL_COUNT=$(count_nul_in_file "$TMPOUT")
[ "$NUL_COUNT" -eq 0 ] && pass "stdout no NUL for single block" || fail "stdout has $NUL_COUNT NUL(s) (expected 0)"
echo ""

# --- Test 3: stdin multi-block → 2 SVGs, 1 NUL separator ---
echo "[3] stdin multi-block → 2 SVGs, 1 NUL separator"
printf 'graph LR\n  A --> B\x00graph TD\n  X --> Y' | "$BINARY" > "$TMPOUT" 2>"$TMPERR"
EXIT=$?
[ $EXIT -eq 0 ] && pass "exit 0" || fail "exit $EXIT (expected 0)"
SVG_COUNT=$(count_svg_in_file "$TMPOUT")
[ "$SVG_COUNT" -eq 2 ] && pass "stdout contains 2 SVGs" || fail "stdout has $SVG_COUNT SVG(s) (expected 2)"
NUL_COUNT=$(count_nul_in_file "$TMPOUT")
[ "$NUL_COUNT" -eq 1 ] && pass "stdout has 1 NUL separator" || fail "stdout has $NUL_COUNT NUL(s) (expected 1)"
echo ""

# --- Test 4: multi-block output order preserved ---
echo "[4] Multi-block output order"
printf 'graph LR\n  Alpha --> Beta\x00graph LR\n  Gamma --> Delta' | "$BINARY" > "$TMPOUT" 2>"$TMPERR"
POS_ALPHA=$(grep -abo 'Alpha' "$TMPOUT" | head -1 | cut -d: -f1 || echo -1)
POS_GAMMA=$(grep -abo 'Gamma' "$TMPOUT" | head -1 | cut -d: -f1 || echo -1)
[ "${POS_ALPHA:--1}" -ge 0 ] && [ "${POS_GAMMA:--1}" -ge 0 ] || {
    fail "Alpha or Gamma not found in stdout (Alpha@$POS_ALPHA, Gamma@$POS_GAMMA)"
    echo ""
}
[ "${POS_ALPHA:--1}" -lt "${POS_GAMMA:--1}" ] \
    && pass "Output order correct: Alpha(block1) < Gamma(block2)" \
    || fail "Output order reversed: Alpha@$POS_ALPHA, Gamma@$POS_GAMMA"
echo ""

# --- Test 5: trailing NUL is ignored ---
echo "[5] Trailing NUL ignored"
printf 'graph LR\n  A --> B\x00' | "$BINARY" > "$TMPOUT" 2>"$TMPERR"
EXIT=$?
[ $EXIT -eq 0 ] && pass "exit 0" || fail "exit $EXIT (expected 0)"
SVG_COUNT=$(count_svg_in_file "$TMPOUT")
[ "$SVG_COUNT" -eq 1 ] && pass "stdout contains 1 SVG (trailing NUL ignored)" || fail "stdout has $SVG_COUNT SVG(s) (expected 1)"
[ ! -s "$TMPERR" ] && pass "stderr empty" || fail "stderr not empty: $(cat "$TMPERR")"
echo ""

# --- Test 6: continue-on-error (block 2 of 3 invalid) ---
echo "[6] Continue-on-error: block 2 of 3 invalid"
printf 'graph LR\n  A --> B\x00thisIsNotValidMermaid\x00graph TD\n  X --> Y' \
    | "$BINARY" --meta > "$TMPOUT" 2>"$TMPERR"
EXIT=$?
[ $EXIT -eq 0 ] && pass "exit 0 (continue-on-error)" || fail "exit $EXIT (expected 0)"
SVG_COUNT=$(count_svg_in_file "$TMPOUT")
[ "$SVG_COUNT" -eq 2 ] && pass "stdout contains 2 SVGs (valid blocks only)" || fail "stdout has $SVG_COUNT SVG(s) (expected 2)"
NUL_COUNT=$(count_nul_in_file "$TMPOUT")
[ "$NUL_COUNT" -eq 1 ] && pass "stdout has 1 NUL separator" || fail "stdout has $NUL_COUNT NUL(s) (expected 1)"
grep -qF '<!-- {"id": 2} -->' "$TMPERR" \
    && pass "stderr contains --meta comment for failed block 2" \
    || fail "stderr missing <!-- {\"id\": 2} -->: $(cat "$TMPERR")"
grep -qF 'No diagram type detected' "$TMPERR" \
    && pass "stderr contains mermaid error text" \
    || fail "stderr missing mermaid error text: $(cat "$TMPERR")"
echo ""

# --- Test 7: all blocks fail → empty stdout, 3 errors on stderr ---
echo "[7] All blocks fail → empty stdout, 3 errors on stderr"
printf 'bogusOne\x00bogusTwo\x00bogusThree' \
    | "$BINARY" --meta > "$TMPOUT" 2>"$TMPERR"
EXIT=$?
[ $EXIT -eq 0 ] && pass "exit 0" || fail "exit $EXIT (expected 0)"
[ ! -s "$TMPOUT" ] && pass "stdout empty" || fail "stdout not empty ($(wc -c < "$TMPOUT") bytes)"
COUNT_IDS=$(grep -c '<!-- {"id":' "$TMPERR" || true)
[ "$COUNT_IDS" -eq 3 ] && pass "stderr has 3 --meta error comments" || fail "stderr has $COUNT_IDS meta comments (expected 3)"
echo ""

# --- Test 8: --meta flag → stdout includes <!-- {"id": N} --> ---
echo "[8] --meta flag → stdout includes <!-- {\"id\": N} -->"
printf 'graph LR\n  A --> B\n' | "$BINARY" --meta > "$TMPOUT" 2>"$TMPERR"
EXIT=$?
[ $EXIT -eq 0 ] && pass "exit 0" || fail "exit $EXIT (expected 0)"
grep -qF '<!-- {"id": 1} -->' "$TMPOUT" \
    && pass 'stdout contains <!-- {"id": 1} -->' \
    || fail "stdout missing <!-- {\"id\": 1} -->: $(head -3 "$TMPOUT")"
echo ""

# --- Test 9: --theme option ---
echo "[9] --theme dark → produces SVG"
MMD=$(mktemp /tmp/sekien_test_XXXXXX.mmd)
printf 'graph LR\n  A --> B\n' > "$MMD"
"$BINARY" --theme dark "$MMD" > "$TMPOUT" 2>"$TMPERR"
EXIT=$?
rm -f "$MMD"
[ $EXIT -eq 0 ] && pass "exit 0" || fail "exit $EXIT (expected 0)"
SVG_COUNT=$(count_svg_in_file "$TMPOUT")
[ "$SVG_COUNT" -gt 0 ] && pass "stdout contains <svg" || fail "stdout missing <svg"
echo ""

# --- Test 10: --font option ---
echo "[10] --font Arial → produces SVG"
MMD=$(mktemp /tmp/sekien_test_XXXXXX.mmd)
printf 'graph LR\n  A --> B\n' > "$MMD"
"$BINARY" --font Arial "$MMD" > "$TMPOUT" 2>"$TMPERR"
EXIT=$?
rm -f "$MMD"
[ $EXIT -eq 0 ] && pass "exit 0" || fail "exit $EXIT (expected 0)"
SVG_COUNT=$(count_svg_in_file "$TMPOUT")
[ "$SVG_COUNT" -gt 0 ] && pass "stdout contains <svg" || fail "stdout missing <svg"
echo ""

# --- Test 11: --look option ---
echo "[11] --look classic → produces SVG"
MMD=$(mktemp /tmp/sekien_test_XXXXXX.mmd)
printf 'graph LR\n  A --> B\n' > "$MMD"
"$BINARY" --look classic "$MMD" > "$TMPOUT" 2>"$TMPERR"
EXIT=$?
rm -f "$MMD"
[ $EXIT -eq 0 ] && pass "exit 0" || fail "exit $EXIT (expected 0)"
SVG_COUNT=$(count_svg_in_file "$TMPOUT")
[ "$SVG_COUNT" -gt 0 ] && pass "stdout contains <svg" || fail "stdout missing <svg"
echo ""

# --- Test 12: --config option ---
echo "[12] --config → produces SVG"
CFGTMP=$(mktemp /tmp/sekien_test_XXXXXX.json)
printf '{"theme":"dark"}' > "$CFGTMP"
printf 'graph LR\n  A --> B\n' | "$BINARY" --config "$CFGTMP" > "$TMPOUT" 2>"$TMPERR"
EXIT=$?
rm -f "$CFGTMP"
[ $EXIT -eq 0 ] && pass "exit 0" || fail "exit $EXIT (expected 0)"
SVG_COUNT=$(count_svg_in_file "$TMPOUT")
[ "$SVG_COUNT" -gt 0 ] && pass "stdout contains <svg" || fail "stdout missing <svg"
echo ""

# --- Test 13: multiple files → non-zero exit ---
echo "[13] Multiple files → non-zero exit"
MMD1=$(mktemp /tmp/sekien_test_XXXXXX.mmd)
MMD2=$(mktemp /tmp/sekien_test_XXXXXX.mmd)
printf 'graph LR\n  A --> B\n' > "$MMD1"
printf 'graph TD\n  X --> Y\n' > "$MMD2"
"$BINARY" "$MMD1" "$MMD2" > "$TMPOUT" 2>"$TMPERR"
EXIT=$?
rm -f "$MMD1" "$MMD2"
[ $EXIT -ne 0 ] && pass "exit non-0 ($EXIT)" || fail "exit 0 (expected non-0)"
echo ""

# --- Test 14: --version ---
echo "[14] --version → contains 'sekien' and 'mermaid'"
"$BINARY" --version > "$TMPOUT" 2>"$TMPERR"
EXIT=$?
[ $EXIT -eq 0 ] && pass "exit 0" || fail "exit $EXIT (expected 0)"
grep -qF 'sekien'  "$TMPOUT" && pass "stdout contains 'sekien'"  || fail "stdout missing 'sekien'"
grep -qF 'mermaid' "$TMPOUT" && pass "stdout contains 'mermaid'" || fail "stdout missing 'mermaid'"
echo ""

# --- Test 15: --help ---
echo "[15] --help → exit 0"
"$BINARY" --help > "$TMPOUT" 2>"$TMPERR"
EXIT=$?
[ $EXIT -eq 0 ] && pass "exit 0" || fail "exit $EXIT (expected 0)"
echo ""

# --- Test 16: whitespace after NUL is not treated as an empty block ---
echo "[16] Whitespace after NUL → ignored"
printf 'graph LR\n  A --> B\x00\n' | "$BINARY" > "$TMPOUT" 2>"$TMPERR"
EXIT=$?
[ $EXIT -eq 0 ] && pass "exit 0" || fail "exit $EXIT (expected 0)"
SVG_COUNT=$(count_svg_in_file "$TMPOUT")
[ "$SVG_COUNT" -eq 1 ] && pass "stdout contains 1 SVG" || fail "stdout has $SVG_COUNT SVG(s) (expected 1)"
[ ! -s "$TMPERR" ] && pass "stderr empty" || fail "stderr not empty: $(cat "$TMPERR")"
echo ""

# --- Results ---
if [ "$FAILED" -eq 0 ]; then
    echo "=== ALL PASSED ==="
else
    echo "=== $FAILED FAILED ==="
    exit 1
fi
