#!/usr/bin/env bash
# Build Anvil's in-tree C++ units with ASan+UBSan, and (when nightly is
# available) run a Rust ASan smoke on the workspace. Exits non-zero with a
# clear message when a needed toolchain is missing.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

MODE="${1:-all}"
if [[ -n "${ANVIL_SANITIZE_OUT:-}" ]]; then
  OUT_DIR="$ANVIL_SANITIZE_OUT"
elif [[ -n "${CARGO_TARGET_DIR:-}" ]]; then
  OUT_DIR="$CARGO_TARGET_DIR/sanitize"
else
  OUT_DIR="$ROOT/target/sanitize"
fi
mkdir -p "$OUT_DIR"

have_cmd() { command -v "$1" >/dev/null 2>&1; }

need() {
  echo "sanitize.sh: missing $1" >&2
  echo "sanitize.sh: install it, then re-run (this script does not install toolchains)." >&2
  exit 2
}

run_cpp() {
  if ! have_cmd g++; then
    need "g++ (C++20 compiler with -fsanitize=address,undefined)"
  fi
  if ! echo 'int main(){return 0;}' | g++ -x c++ -std=c++20 -fsanitize=address,undefined -c -o /dev/null - 2>/dev/null; then
    echo "sanitize.sh: g++ rejects -fsanitize=address,undefined (libasan/libubsan missing?)" >&2
    exit 2
  fi

  # Same units the crate build.rs files compile on Linux.
  local -a units=(
    "engines/cinder/cpp/cinder_jit_native.cpp|-Iengines/cinder/cpp"
    "engines/cinder/cpp/cinder_vm_compiler.cpp|-Iengines/cinder/cpp"
    "engines/cinder/cpp/cinder_vm_ic.cpp|-Iengines/cinder/cpp"
    "engines/cinder/cpp/cinder_vm_heap.cpp|-Iengines/cinder/cpp"
    "engines/cinder/cpp/cinder_jit_accel.cpp|-Iengines/cinder/cpp"
    "engines/prism/cpp/prism_css_engine.cpp|-Iengines/prism/cpp"
    "crates/anvil-raster/cpp/forge_raster_accel.cpp|-Icrates/anvil-raster/cpp"
    "crates/anvil-raster/cpp/forge_raster_compositor.cpp|-Icrates/anvil-raster/cpp"
    "ui/anvil_ui_d2d.cpp|-Iui"
    "ui/anvil_ui_window.cpp|-Iui"
  )

  local failed=0
  local src flags stem obj
  for entry in "${units[@]}"; do
    src="${entry%%|*}"
    flags="${entry#*|}"
    if [[ ! -f "$src" ]]; then
      echo "sanitize.sh: skip missing $src"
      continue
    fi
    stem="$(basename "$src" .cpp)"
    obj="$OUT_DIR/${stem}.asan.o"
    echo "sanitize.sh: g++ -fsanitize=address,undefined $src"
    # Match build.rs: C++20, no exceptions/RTTI. Add sanitize flags.
    if ! g++ -c -O1 -g -std=c++20 -fno-exceptions -fno-rtti \
        -fsanitize=address,undefined -fno-omit-frame-pointer \
        $flags "$src" -o "$obj"; then
      echo "sanitize.sh: C++ sanitize compile failed for $src" >&2
      failed=1
    fi
  done
  if [[ "$failed" -ne 0 ]]; then
    exit 1
  fi
  echo "sanitize.sh: C++ ASan/UBSan compile ok ($(ls -1 "$OUT_DIR"/*.asan.o 2>/dev/null | wc -l) objects)"
}

run_rust() {
  if ! have_cmd rustup; then
    need "rustup (to provide a nightly toolchain for -Zsanitizer)"
  fi
  if ! rustup run nightly rustc -Z help 2>/dev/null | grep -q sanitizer; then
    # Try installing nothing; just report.
    if ! rustup toolchain list | grep -q '^nightly'; then
      need "nightly Rust toolchain (rustup toolchain install nightly)"
    fi
    echo "sanitize.sh: nightly rustc lacks -Zsanitizer support" >&2
    exit 2
  fi

  HOST="$(rustup run nightly rustc -vV | awk '/^host:/{print $2}')"
  # Proc-macro crates are host builds. Rust rejects -Zsanitizer on them, so a
  # global RUSTFLAGS sanitizer stops the smoke before any test runs. Flags go
  # on the target only. libstd is rebuilt with the same sanitizer so the tests
  # link and run instrumented code. This is the runtime smoke. The C++ step
  # above only compiles translation units.
  unset RUSTFLAGS
  export RUSTDOCFLAGS="-Zsanitizer=address -Cforce-frame-pointers=yes"
  export RUST_MIN_STACK="${RUST_MIN_STACK:-33554432}"
  echo "sanitize.sh: cargo +nightly test -Zbuild-std --target $HOST (address sanitizer)"
  ERR="$OUT_DIR/rust-asan.err"
  if rustup run nightly cargo -Zbuild-std test --locked --workspace --target "$HOST" \
      --config "target.${HOST}.rustflags=[\"-Zsanitizer=address\", \"-Cforce-frame-pointers=yes\"]" \
      >"$OUT_DIR/rust-asan.out" 2>"$ERR"; then
    echo "sanitize.sh: Rust ASan smoke ok"
    return 0
  fi
  echo "sanitize.sh: Rust ASan run failed; see $ERR" >&2
  tail -n 80 "$ERR" >&2 || true
  exit 1
}

case "$MODE" in
  cpp) run_cpp ;;
  rust) run_rust ;;
  all)
    run_cpp
    run_rust
    ;;
  *)
    echo "usage: $0 [all|cpp|rust]" >&2
    exit 2
    ;;
esac
