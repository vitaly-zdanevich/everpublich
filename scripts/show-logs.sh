#!/usr/bin/env bash
set -euo pipefail

PROJECT_NAME="${PROJECT_NAME:-everpublich}"
AWS_REGION="${AWS_REGION:-${AWS_DEFAULT_REGION:-us-east-1}}"
FUNCTION_NAME="${1:-${PROJECT_NAME}-api}"
MINUTES="${LOG_MINUTES:-30}"
START_TIME="$(( ($(date +%s) - MINUTES * 60) * 1000 ))"
LOG_GROUP="/aws/lambda/$FUNCTION_NAME"

echo "Reading $LOG_GROUP in $AWS_REGION" >&2

aws logs filter-log-events \
	--region "$AWS_REGION" \
	--log-group-name "$LOG_GROUP" \
	--start-time "$START_TIME" \
	--interleaved \
	--query 'events[*].message' \
	--output text
