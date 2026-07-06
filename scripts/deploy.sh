#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TERRAFORM_DIR="$ROOT_DIR/infra"
AUTO_APPROVE="${TERRAFORM_AUTO_APPROVE:-1}"
STATE_FILE="$TERRAFORM_DIR/terraform.tfstate"

if [[ -f "$STATE_FILE" ]] && grep -q 'oracle/oci' "$STATE_FILE"; then
	echo 'Old OCI Terraform state found at infra/terraform.tfstate.' >&2
	echo 'Move it aside before applying the AWS EC2 stack:' >&2
	echo 'mv infra/terraform.tfstate infra/terraform.tfstate.oci' >&2
	echo 'mv infra/terraform.tfstate.backup infra/terraform.tfstate.oci.backup' >&2
	exit 2
fi

apply_args=()
if [[ "$AUTO_APPROVE" == "1" ]]; then
	apply_args=(-auto-approve)
fi

terraform -chdir="$TERRAFORM_DIR" init
terraform -chdir="$TERRAFORM_DIR" apply "${apply_args[@]}" "$@"
terraform -chdir="$TERRAFORM_DIR" output
