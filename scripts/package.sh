#!/usr/bin/env bash
# Package only the CLI; fixture binaries are never distributed.
set -euo pipefail
if [[ $# -ne 1 ]]; then
  echo "usage: $0 <target-triple>" >&2
  exit 2
fi
target=$1
case "$target" in
  x86_64-unknown-linux-musl|aarch64-unknown-linux-musl|aarch64-apple-darwin|x86_64-apple-darwin) ;;
  *) echo "unsupported target: $target" >&2; exit 2 ;;
esac
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
name="tinyagentaccess-$target"
staging=$(mktemp -d)
trap 'rm -rf "$staging"' EXIT
mkdir -p "$staging/$name" dist
cp "target/$target/release/tinyagentaccess" "$staging/$name/"
cp LICENSE README.md THIRD_PARTY_NOTICES.md "$staging/$name/"
# Stable member order and permissions; tar metadata follows the source build.
chmod 755 "$staging/$name" "$staging/$name/tinyagentaccess"
chmod 644 "$staging/$name/"*.md "$staging/$name/LICENSE"
COPYFILE_DISABLE=1 tar -czf "dist/$name.tar.gz" -C "$staging" \
  "$name/tinyagentaccess" "$name/LICENSE" "$name/README.md" "$name/THIRD_PARTY_NOTICES.md"
