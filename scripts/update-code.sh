#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SSH_HOST="${EVERPUBLICH_SSH_HOST:-${1:-}}"
SSH_USER="${EVERPUBLICH_SSH_USER:-ubuntu}"
SSH_KEY="${EVERPUBLICH_SSH_KEY:-}"
BUILD_MODE="${EVERPUBLICH_BUILD_MODE:-docker}"
BUILD_IMAGE="${EVERPUBLICH_BUILD_IMAGE:-rust:1.95-bookworm}"
BINARY_PATH="${EVERPUBLICH_BINARY_PATH:-$ROOT_DIR/target/release/everpublich-cli}"
RUSTFLAGS_DEFAULT="${EVERPUBLICH_RUSTFLAGS--C target-cpu=sapphirerapids}"

if [[ -z "$SSH_HOST" ]]; then
	echo 'Usage: EVERPUBLICH_SSH_HOST=PUBLIC_IP scripts/update-code.sh [PUBLIC_IP]' >&2
	exit 2
fi

ssh_args=()
if [[ -n "$SSH_KEY" ]]; then
	ssh_args=(-i "$SSH_KEY")
fi

case "$BUILD_MODE" in
	docker)
		mkdir -p "$ROOT_DIR/target/docker-cargo-home"
		docker run --rm \
			--user "$(id -u):$(id -g)" \
			-e CARGO_HOME=/cargo-home \
			-e RUSTFLAGS="$RUSTFLAGS_DEFAULT" \
			-e PATH=/usr/local/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
			-v "$ROOT_DIR:/work" \
			-v "$ROOT_DIR/target/docker-cargo-home:/cargo-home" \
			-w /work \
			"$BUILD_IMAGE" \
			bash -lc 'toolchain="$(/usr/local/cargo/bin/rustup toolchain list | awk "NR == 1 { print \$1 }")"; export PATH="/usr/local/rustup/toolchains/$toolchain/bin:$PATH"; /usr/local/cargo/bin/rustup run "$toolchain" cargo build --release --locked --bin everpublich-cli'
		;;
	local)
		(
			cd "$ROOT_DIR"
			export RUSTFLAGS="$RUSTFLAGS_DEFAULT"
			cargo build --release --locked --bin everpublich-cli
		)
		;;
	skip)
		;;
	*)
		echo "Unknown EVERPUBLICH_BUILD_MODE: $BUILD_MODE" >&2
		exit 2
		;;
esac

if [[ ! -x "$BINARY_PATH" ]]; then
	echo "Built binary not found or not executable: $BINARY_PATH" >&2
	exit 2
fi

remote_tmp="/tmp/everpublich-cli.$RANDOM.$$"
scp "${ssh_args[@]}" "$BINARY_PATH" "$SSH_USER@$SSH_HOST:$remote_tmp"

ssh "${ssh_args[@]}" "$SSH_USER@$SSH_HOST" "set -euo pipefail
sudo install -o root -g root -m 0755 '$remote_tmp' /opt/everpublich/bin/everpublich-cli
rm -f '$remote_tmp'
sudo systemctl start everpublich-sync.service
"
