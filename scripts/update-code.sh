#!/usr/bin/env bash
set -euo pipefail

SSH_HOST="${EVERPUBLICH_SSH_HOST:-${1:-}}"
SSH_USER="${EVERPUBLICH_SSH_USER:-ubuntu}"
SSH_KEY="${EVERPUBLICH_SSH_KEY:-}"

if [[ -z "$SSH_HOST" ]]; then
	echo 'Usage: EVERPUBLICH_SSH_HOST=PUBLIC_IP scripts/update-code.sh [PUBLIC_IP]' >&2
	exit 2
fi

ssh_args=()
if [[ -n "$SSH_KEY" ]]; then
	ssh_args=(-i "$SSH_KEY")
fi

ssh "${ssh_args[@]}" "$SSH_USER@$SSH_HOST" 'bash -s' <<'REMOTE'
set -euo pipefail

sudo -H -u everpublich bash -lc '
	set -euo pipefail
	cd /opt/everpublich/repo
	git pull --ff-only
	if [ -f /var/lib/everpublich/.cargo/env ]; then
		. /var/lib/everpublich/.cargo/env
	fi
	cargo build --release --bin everpublich-cli
'

sudo install -o root -g root -m 0755 \
	/opt/everpublich/repo/target/release/everpublich-cli \
	/opt/everpublich/bin/everpublich-cli

sudo systemctl start everpublich-sync.service
REMOTE
