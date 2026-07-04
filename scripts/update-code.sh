#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
AWS_REGION="${AWS_REGION:-${AWS_DEFAULT_REGION:-us-east-1}}"
PROJECT_NAME="${PROJECT_NAME:-everpublich}"
API_ZIP="$ROOT_DIR/build/everpublich-lambda.zip"
WORKER_ZIP="$ROOT_DIR/build/everpublich-worker.zip"

"$ROOT_DIR/scripts/build-lambda.sh"

aws lambda update-function-code \
	--region "$AWS_REGION" \
	--function-name "${PROJECT_NAME}-api" \
	--zip-file "fileb://$API_ZIP" \
	>/dev/null

aws lambda wait function-updated \
	--region "$AWS_REGION" \
	--function-name "${PROJECT_NAME}-api"

for function_name in $(aws lambda list-functions --region "$AWS_REGION" --query "Functions[?starts_with(FunctionName, '${PROJECT_NAME}-builder-')].FunctionName" --output text); do
	aws lambda update-function-code \
		--region "$AWS_REGION" \
		--function-name "$function_name" \
		--zip-file "fileb://$WORKER_ZIP" \
		>/dev/null
	aws lambda wait function-updated --region "$AWS_REGION" --function-name "$function_name"
	echo "Updated $function_name"
done

echo "Updated ${PROJECT_NAME}-api and builder Lambdas in $AWS_REGION"
