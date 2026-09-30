#!/bin/bash
# Update the bundled mermaid.js to a given npm release.
#
# Usage:
#   ./update-mermaid.sh <version>        # e.g. ./update-mermaid.sh 12.0.0
#
# Downloads mermaid@<version> from the npm registry, verifies the tarball
# against the registry's sha512 integrity, and then:
#   - replaces assets/mermaid/ with the tarball's ESM build: the entry
#     dist/mermaid.esm.min.mjs and its chunks dist/chunks/mermaid.esm.min/
#     (without source maps), keeping the dist layout so relative imports work
#   - writes <version> to assets/mermaid.version (read by build.rs)
#   - rewrites EXPECTED_MERMAID_SHA in build.rs (the manifest hash below)
#
# The version is taken from the tarball's package.json rather than parsed out
# of the minified bundle, which has no stable, structured version marker.
#
# assets/mermaid.LICENSE is not touched; review it by hand if needed.
#
# Dependencies: curl, tar, openssl, base64, sha256sum, python3
set -euo pipefail

if [ $# -ne 1 ]; then
    echo "usage: $0 <version>" >&2
    exit 2
fi
VERSION="$1"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
WORK=$(mktemp -d /tmp/sekien_mermaid_XXXXXX)
cleanup() { rm -rf "$WORK"; }
trap cleanup EXIT

echo "Fetching mermaid@$VERSION metadata..." >&2
curl -sfL "https://registry.npmjs.org/mermaid/$VERSION" -o "$WORK/meta.json" \
    || { echo "error: mermaid@$VERSION not found on npm" >&2; exit 1; }
read -r TARBALL INTEGRITY < <(python3 -c '
import json, sys
d = json.load(open(sys.argv[1]))["dist"]
print(d["tarball"], d["integrity"])
' "$WORK/meta.json")

echo "Downloading $TARBALL..." >&2
curl -sfL "$TARBALL" -o "$WORK/mermaid.tgz"

ACTUAL_INTEGRITY="sha512-$(openssl dgst -sha512 -binary "$WORK/mermaid.tgz" | base64 -w0)"
if [ "$ACTUAL_INTEGRITY" != "$INTEGRITY" ]; then
    echo "error: tarball integrity mismatch" >&2
    echo "  expected: $INTEGRITY" >&2
    echo "  actual:   $ACTUAL_INTEGRITY" >&2
    exit 1
fi

tar xzf "$WORK/mermaid.tgz" -C "$WORK"
PKG_VERSION=$(python3 -c 'import json, sys; print(json.load(open(sys.argv[1]))["version"])' \
    "$WORK/package/package.json")
if [ "$PKG_VERSION" != "$VERSION" ]; then
    echo "error: package.json says $PKG_VERSION, expected $VERSION" >&2
    exit 1
fi

DEST="$REPO_ROOT/assets/mermaid"
rm -rf "$DEST"
mkdir -p "$DEST/chunks/mermaid.esm.min"
cp "$WORK/package/dist/mermaid.esm.min.mjs" "$DEST/"
cp "$WORK/package/dist/chunks/mermaid.esm.min/"*.mjs "$DEST/chunks/mermaid.esm.min/"
printf '%s\n' "$VERSION" > "$REPO_ROOT/assets/mermaid.version"

# Manifest hash, computed the same way by build.rs: for each file in byte
# order of its path, "<sha256 of the content with \r stripped>  <path>\n",
# then the sha256 of all those lines.
SHA=$(cd "$DEST" && find . -type f -name '*.mjs' | sed 's|^\./||' | LC_ALL=C sort \
    | while read -r f; do
        printf '%s  %s\n' "$(tr -d '\r' < "$f" | sha256sum | cut -d' ' -f1)" "$f"
    done | sha256sum | cut -d' ' -f1)
sed -i -E "s/^(    \")[0-9a-f]{64}(\";)$/\1$SHA\2/" "$REPO_ROOT/build.rs"
grep -q "\"$SHA\"" "$REPO_ROOT/build.rs" \
    || { echo "error: failed to update EXPECTED_MERMAID_SHA in build.rs" >&2; exit 1; }

echo "mermaid.js updated to $VERSION" >&2
echo "  files: $(find "$DEST" -type f -name '*.mjs' | wc -l), manifest sha256: $SHA" >&2
