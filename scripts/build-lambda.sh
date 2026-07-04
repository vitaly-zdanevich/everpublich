#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TOOLS_DIR="$ROOT_DIR/.tools"
TARGET="aarch64-unknown-linux-gnu"
RUST_TARGET_CPU="${RUST_TARGET_CPU:-neoverse-n1}"
BUILD_DIR="$ROOT_DIR/build/lambda"
API_ZIP="$ROOT_DIR/build/everpublich-lambda.zip"
WORKER_ZIP="$ROOT_DIR/build/everpublich-worker.zip"

mkdir -p "$BUILD_DIR" "$ROOT_DIR/build"

if ! rustup target list --installed | grep -qx "$TARGET"; then
	echo "Rust target $TARGET not found; installing it with rustup"
	rustup target add "$TARGET"
fi

export PATH="$TOOLS_DIR/bin:$PATH"

if ! cargo lambda --version >/dev/null 2>&1; then
	echo "cargo-lambda not found; installing it into $TOOLS_DIR"
	cargo install cargo-lambda --root "$TOOLS_DIR"
fi

export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUSTFLAGS="${CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUSTFLAGS:-} -C target-cpu=$RUST_TARGET_CPU"

echo "Building Everpublich Lambdas for $TARGET with target-cpu=$RUST_TARGET_CPU"
cargo lambda build \
	--release \
	--arm64 \
	--lambda-dir "$BUILD_DIR" \
	--output-format zip \
	--bin everpublich-lambda \
	--bin everpublich-worker

cp "$BUILD_DIR/everpublich-lambda/bootstrap.zip" "$API_ZIP"
cp "$BUILD_DIR/everpublich-worker/bootstrap.zip" "$WORKER_ZIP"

printf 'API Lambda zip: %s\n' "$API_ZIP"
printf 'Worker Lambda zip: %s\n' "$WORKER_ZIP"
