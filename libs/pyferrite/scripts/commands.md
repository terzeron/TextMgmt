# The raw commands

`scripts/bootstrap.sh` runs all of these in order. This page is for when you
want to do one thing at a time.

## Set up the toolchain

```bash
# Install Rust (any platform)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"

# Or from a distribution's packages
sudo apt-get install -y rustc cargo     # Debian, Ubuntu
sudo dnf install -y rust cargo          # Fedora
brew install rust                       # macOS

# Optional components used by the script
rustup component add rustfmt clippy
```

## Start a project from scratch

```bash
cargo new --lib pyferrite
cd pyferrite
mkdir -p src tests docs/en docs/ru examples scripts
```

## Build

```bash
cargo build                        # debug, default features
cargo build --release              # optimised, with LTO
cargo build --no-default-features  # no compression, ndarray only
cargo build --all-features         # includes the polars bridge
```

## Test

```bash
cargo test                    # everything
cargo test --test formats     # one integration file
cargo test --doc              # the examples in the documentation
cargo test -- --nocapture     # show println! output
cargo test roundtrip          # only tests whose name matches
```

## Format and lint

```bash
cargo fmt --all
cargo fmt --all -- --check    # verify without rewriting, for CI
cargo clippy --all-targets -- -D warnings
```

## Documentation

```bash
cargo doc --no-deps --open
```

## Cross-validate against Python

```bash
pip install numpy joblib h5py
python3 scripts/crossvalidate.py
```

## Package and publish

```bash
cargo search pyferrite        # is the name still free?
cargo package                 # build the .crate archive
cargo package --list          # what would be included
cargo publish --dry-run       # full rehearsal
cargo login <api-token>       # once, from https://crates.io/me
cargo publish                 # for real; a published version is permanent
```

A published version can never be overwritten or deleted, only yanked, so run
the dry run first.

## Use it from another project

```bash
cargo add pyferrite
cargo add pyferrite --features polars-interop
cargo add pyferrite --no-default-features
```
