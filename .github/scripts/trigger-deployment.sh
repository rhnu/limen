#!/usr/bin/env bash
# CI invocation is commented out during the persistence/management refactor.
# Retained for explicitly requested deployment restoration; do not run during refactoring.
set -euo pipefail

for variable in CF_ACCESS_CLIENT_ID CF_ACCESS_CLIENT_SECRET WEBHOOK_TOKEN WEBHOOK_URL \
  DEPLOY_IMAGE GITHUB_REPOSITORY GITHUB_REF GITHUB_SHA GITHUB_RUN_ID GITHUB_RUN_ATTEMPT; do
  if [[ -z "${!variable:-}" ]]; then
    echo "::error::Required deployment variable ${variable} is empty"
    exit 1
  fi
done

if [[ "$WEBHOOK_URL" != https://* ]]; then
  echo '::error::WEBHOOK_URL must use HTTPS and point to the OliveTin /webhooks endpoint'
  exit 1
fi

request_id="limen-${GITHUB_RUN_ID}-${GITHUB_RUN_ATTEMPT}"
# OliveTin parses JSON even when the action has no extracted arguments.
payload=$(jq -cn \
  --arg target limen \
  --arg repository "$GITHUB_REPOSITORY" \
  --arg ref "$GITHUB_REF" \
  --arg sha "$GITHUB_SHA" \
  --arg image "$DEPLOY_IMAGE" \
  --arg request_id "$request_id" \
  '{target: $target, repository: $repository, ref: $ref, sha: $sha, image: $image, request_id: $request_id}')

response_file=$(mktemp)
trap 'rm -f -- "$response_file"' EXIT
curl_exit=0
status=$(curl --disable --fail-with-body --silent --show-error \
  --proto '=https' --connect-timeout 10 --max-time 60 \
  --retry 2 --retry-connrefused --retry-max-time 120 \
  --request POST \
  --header "CF-Access-Client-Id: ${CF_ACCESS_CLIENT_ID}" \
  --header "CF-Access-Client-Secret: ${CF_ACCESS_CLIENT_SECRET}" \
  --header "Authorization: Bearer ${WEBHOOK_TOKEN}" \
  --header 'Content-Type: application/json' \
  --header 'Accept: text/plain' \
  --header 'X-Deploy: limen' \
  --header "X-Request-Id: ${request_id}" \
  --data-binary "$payload" \
  --output "$response_file" --write-out '%{http_code}' \
  "$WEBHOOK_URL") || curl_exit=$?

if (( curl_exit != 0 )); then
  echo "::error::Deployment webhook failed: curl exit ${curl_exit}, HTTP ${status:-000}, request ${request_id}"
  exit 1
fi

# Cloudflare login/error pages can return 200; OliveTin's webhook response is OK.
# OliveTin also returns OK for unmatched/rejected actions, so this is receipt only.
if [[ "$status" != 200 || "$(<"$response_file")" != OK ]]; then
  echo "::error::Unexpected OliveTin webhook response: HTTP ${status:-000}, request ${request_id}"
  exit 1
fi

echo "OliveTin webhook acknowledged (HTTP 200, request ${request_id}); check OliveTin for action execution"
