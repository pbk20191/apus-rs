#!/bin/bash
# Builds examples/swift_basic with cross-language LTO (feature xlto) and checks that the C<->Rust boundary was
# inlined (the step funclet no longer calls the poll callback) and left no boundary symbol in the export list.
# Needs an LLVM whose major version matches rustc's: APUS_LLVM (default: Homebrew's). Skips without one.
set -euo pipefail
cd "$(dirname "$0")/.."
LLVM="${APUS_LLVM:-/opt/homebrew/opt/llvm}"
if [[ ! -x "$LLVM/bin/clang" ]]; then
  echo "check-xlto: skipped, no clang at $LLVM/bin/clang (set APUS_LLVM)"
  exit 0
fi
target="$(rustc -vV | sed -n 's/^host: //p')"
cc_var="CC_${target//-/_}"
export APUS_LLVM="$LLVM" "$cc_var=$LLVM/bin/clang"
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-14.0}"
export CARGO_PROFILE_RELEASE_LTO=fat CARGO_TARGET_DIR=target/xlto
export RUSTFLAGS="-Clinker-plugin-lto -Clinker=$PWD/scripts/xlto-link"
cargo build -q --release --target "$target" --features xlto --example swift_basic
bin="target/xlto/$target/release/examples/swift_basic"
"$bin" > /dev/null
exported="$(xcrun dyld_info -exports "$bin" | grep -cE ' _(apus_|rs_)' || true)"
step="$(xcrun objdump -d --no-show-raw-insn --disassemble-symbols=_step "$bin")"
if [[ "$(grep -c . <<< "$step")" -lt 20 ]]; then
  echo "check-xlto: FAILED (no step funclet found to inspect)"
  exit 1
fi
calls="$(grep -cE '<_apus_[0-9a-f_]+_rs_poll>' <<< "$step" || true)"
if [[ "$exported" != 0 || "$calls" != 0 ]]; then
  echo "check-xlto: FAILED ($exported boundary symbols exported, $calls calls from step to rs_poll)"
  exit 1
fi
echo "check-xlto: ok (boundary inlined, no boundary symbol exported)"
