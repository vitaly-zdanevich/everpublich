#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TERRAFORM_DIR="$ROOT_DIR/infra"
AUTO_APPROVE="${TERRAFORM_AUTO_APPROVE:-1}"
STATE_FILE="$TERRAFORM_DIR/terraform.tfstate"

if [[ -f "$STATE_FILE" ]] && grep -q 'hashicorp/aws' "$STATE_FILE"; then
	echo 'Old AWS Terraform state found at infra/terraform.tfstate.' >&2
	echo 'Move it aside before applying the OCI VM stack:' >&2
	echo 'mv infra/terraform.tfstate infra/terraform.tfstate.aws' >&2
	echo 'mv infra/terraform.tfstate.backup infra/terraform.tfstate.aws.backup' >&2
	exit 2
fi

apply_args=()
if [[ "$AUTO_APPROVE" == "1" ]]; then
	apply_args=(-auto-approve)
fi

terraform -chdir="$TERRAFORM_DIR" init
terraform -chdir="$TERRAFORM_DIR" apply "${apply_args[@]}" "$@"
terraform -chdir="$TERRAFORM_DIR" output
