#!/usr/bin/env bash
set -Eeuo pipefail

# Non-destructive API smoke test. Fixtures (user token, project and payload)
# are supplied by the isolated e2e environment rather than checked into git.
: "${TASKTIPS_E2E_API_URL:?TASKTIPS_E2E_API_URL is required}"
: "${TASKTIPS_E2E_ACCESS_TOKEN:?TASKTIPS_E2E_ACCESS_TOKEN is required}"
: "${TASKTIPS_E2E_PROJECT_ID:?TASKTIPS_E2E_PROJECT_ID is required}"
: "${TASKTIPS_E2E_PAYLOAD_FILE:?TASKTIPS_E2E_PAYLOAD_FILE is required}"

payload_hash=$(sha256sum "$TASKTIPS_E2E_PAYLOAD_FILE" | awk '{print $1}')
payload_size=$(wc -c < "$TASKTIPS_E2E_PAYLOAD_FILE")
headers=(
  -H "Authorization: Bearer $TASKTIPS_E2E_ACCESS_TOKEN"
  -H 'Content-Type: application/octet-stream'
  -H "Content-Length: $payload_size"
)
curl --fail --silent --show-error \
  "${headers[@]}" \
  --upload-file "$TASKTIPS_E2E_PAYLOAD_FILE" \
  "$TASKTIPS_E2E_API_URL/api/v1/projects/$TASKTIPS_E2E_PROJECT_ID/payloads/$payload_hash" >/dev/null

download=$(mktemp)
trap 'rm -f -- "$download"' EXIT
curl --fail --silent --show-error \
  -H "Authorization: Bearer $TASKTIPS_E2E_ACCESS_TOKEN" \
  "$TASKTIPS_E2E_API_URL/api/v1/projects/$TASKTIPS_E2E_PROJECT_ID/payloads/$payload_hash" \
  -o "$download"
download_hash=$(sha256sum "$download" | awk '{print $1}')
if [[ "$download_hash" != "$payload_hash" ]]; then
  printf '%s\n' 'payload hash mismatch in e2e smoke test' >&2
  exit 1
fi

curl --fail --silent --show-error \
  -H "Authorization: Bearer $TASKTIPS_E2E_ACCESS_TOKEN" \
  "$TASKTIPS_E2E_API_URL/api/v1/projects/$TASKTIPS_E2E_PROJECT_ID" >/dev/null
printf 'sync e2e smoke test passed\n'
