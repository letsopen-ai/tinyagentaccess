#!/usr/bin/env bash
# Install cargo-about 0.9.3 before running this script.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
cargo about generate --offline --locked -m crates/tinyagentaccess/Cargo.toml \
  --fail -o THIRD_PARTY_LICENSES.html about.hbs
