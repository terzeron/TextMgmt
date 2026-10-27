#!/usr/bin/env bash
#
# pyferrite — one-shot setup, build, test and packaging.
#
#   ./scripts/bootstrap.sh              full run
#   ./scripts/bootstrap.sh --no-python  skip the Python cross-validation
#   ./scripts/bootstrap.sh --publish    also run `cargo publish --dry-run`
#
# Safe to re-run: every step is idempotent.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

RUN_PYTHON=1
RUN_PUBLISH=0
for arg in "$@"; do
    case "$arg" in
        --no-python) RUN_PYTHON=0 ;;
        --publish)   RUN_PUBLISH=1 ;;
        -h|--help)   sed -n '2,10p' "$0"; exit 0 ;;
        *) echo "unknown option: $arg" >&2; exit 2 ;;
    esac
done

step() { printf '\n\033[1;34m==>\033[0m \033[1m%s\033[0m\n' "$1"; }
warn() { printf '\033[1;33mwarning:\033[0m %s\n' "$1"; }
die()  { printf '\033[1;31merror:\033[0m %s\n' "$1" >&2; exit 1; }

# ---------------------------------------------------------------- toolchain

step "Checking the Rust toolchain"
if ! command -v cargo >/dev/null 2>&1; then
    warn "cargo not found; installing via rustup"
    if command -v curl >/dev/null 2>&1; then
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
        # shellcheck disable=SC1091
        . "$HOME/.cargo/env"
    else
        die "neither cargo nor curl is available; install Rust from https://rustup.rs"
    fi
fi
cargo --version
rustc --version

MIN_MINOR=70
ACTUAL_MINOR="$(rustc --version | sed -E 's/rustc 1\.([0-9]+).*/\1/')"
if [ "$ACTUAL_MINOR" -lt "$MIN_MINOR" ]; then
    die "pyferrite needs Rust 1.$MIN_MINOR or newer; found 1.$ACTUAL_MINOR"
fi

# ------------------------------------------------------------------- layout

step "Verifying the project layout"
for required in Cargo.toml LICENSE README.md src/lib.rs; do
    [ -e "$required" ] || die "missing $required — run this from a checkout of the repository"
done
for d in src tests docs/en docs/ru examples; do
    [ -d "$d" ] || warn "expected directory $d is missing"
done
printf '  %s source files, %s test files\n' \
    "$(find src -name '*.rs' | wc -l | tr -d ' ')" \
    "$(find tests -name '*.rs' | wc -l | tr -d ' ')"

# ------------------------------------------------------------- dependencies

step "Fetching dependencies"
cargo fetch --locked 2>/dev/null || cargo fetch

# --------------------------------------------------------------- formatting

step "Formatting"
if cargo fmt --version >/dev/null 2>&1; then
    cargo fmt --all
    echo "  rustfmt applied"
else
    warn "rustfmt is not installed (rustup component add rustfmt); skipping"
fi

# ------------------------------------------------------------------- checks

step "Building (default features)"
cargo build --all-targets

step "Building (no default features — no compression, no dependencies beyond ndarray)"
cargo build --no-default-features

step "Linting"
if cargo clippy --version >/dev/null 2>&1; then
    cargo clippy --all-targets -- -D warnings || warn "clippy reported issues"
else
    warn "clippy is not installed (rustup component add clippy); skipping"
fi

step "Running the test suite"
cargo test --all-targets

step "Running documentation tests"
cargo test --doc

step "Building the API documentation"
cargo doc --no-deps

# ------------------------------------------------- Python cross-validation

if [ "$RUN_PYTHON" -eq 1 ]; then
    step "Cross-validating against Python"
    if command -v python3 >/dev/null 2>&1 && python3 -c 'import numpy' 2>/dev/null; then
        python3 scripts/crossvalidate.py
    else
        warn "python3 with numpy is unavailable; skipping cross-validation"
        warn "install with: pip install numpy joblib h5py"
    fi
fi

# ---------------------------------------------------------------- packaging

step "Packaging for crates.io"
cargo package --allow-dirty
ls -lh target/package/*.crate 2>/dev/null || true

if [ "$RUN_PUBLISH" -eq 1 ]; then
    step "Publish dry run"
    cargo publish --dry-run --allow-dirty
    cat <<'NOTE'

  The dry run succeeded. To publish for real:

      cargo login <your-api-token>
      cargo publish

  Check first that the name is still free:

      cargo search pyferrite

NOTE
fi

step "Done"
echo "  Everything built, tested and packaged."
