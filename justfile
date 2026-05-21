set shell := ["bash", "-euo", "pipefail", "-c"]

default: ci

fmt:
    cargo fmt --check

build:
    RUSTFLAGS="-D warnings" cargo build --all-targets

clippy:
    cargo clippy --all-targets --all-features -- -D warnings

test:
    RUSTFLAGS="-D warnings" cargo test

coverage:
    cargo llvm-cov --workspace --fail-under-lines 80 --fail-under-regions 80

ci: fmt build clippy test coverage
