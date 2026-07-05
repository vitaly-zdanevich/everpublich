#!/usr/bin/env bash
set -euo pipefail

SSH_HOST="${EVERPUBLICH_SSH_HOST:-${1:-}}"
SSH_USER="${EVERPUBLICH_SSH_USER:-ubuntu}"
SSH_KEY="${EVERPUBLICH_SSH_KEY:-}"
SERVICE="${EVERPUBLICH_SERVICE:-${2:-everpublich-sync.service}}"
LOG_LINES="${LOG_LINES:-200}"

if [[ -z "$SSH_HOST" ]]; then
	echo 'Usage: EVERPUBLICH_SSH_HOST=PUBLIC_IP scripts/show-logs.sh [PUBLIC_IP] [systemd-unit]' >&2
	exit 2
fi

ssh_args=()
if [[ -n "$SSH_KEY" ]]; then
	ssh_args=(-i "$SSH_KEY")
fi

ssh "${ssh_args[@]}" "$SSH_USER@$SSH_HOST" \
	"sudo journalctl -u '$SERVICE' -n '$LOG_LINES' --no-pager"
