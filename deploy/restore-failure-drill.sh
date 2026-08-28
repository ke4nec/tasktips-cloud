#!/usr/bin/env bash
set -Eeuo pipefail

# Run only against an isolated restore-drill database and API. This deliberately
# expires one lease to simulate a crashed worker, then verifies that another
# worker either completes or reports a bounded failure.
: "${TASKTIPS_DRILL_CONFIRM:?set TASKTIPS_DRILL_CONFIRM=YES for an isolated failure drill}"
if [[ "$TASKTIPS_DRILL_CONFIRM" != "YES" ]]; then
  printf '%s\n' 'refusing failure drill without TASKTIPS_DRILL_CONFIRM=YES' >&2
  exit 2
fi
: "${TASKTIPS_DRILL_DATABASE_URL:?set TASKTIPS_DRILL_DATABASE_URL}"
: "${TASKTIPS_DRILL_API_URL:?set TASKTIPS_DRILL_API_URL}"
: "${TASKTIPS_DRILL_ACCESS_TOKEN:?set TASKTIPS_DRILL_ACCESS_TOKEN}"
: "${TASKTIPS_DRILL_PROJECT_ID:?set TASKTIPS_DRILL_PROJECT_ID}"
: "${TASKTIPS_DRILL_RESTORE_ID:?set TASKTIPS_DRILL_RESTORE_ID}"

poll_seconds=${TASKTIPS_DRILL_POLL_SECONDS:-5}
timeout_seconds=${TASKTIPS_DRILL_TIMEOUT_SECONDS:-900}
deadline=$((SECONDS + timeout_seconds))

read_status() {
  curl --fail --silent --show-error \
    -H "Authorization: Bearer $TASKTIPS_DRILL_ACCESS_TOKEN" \
    "$TASKTIPS_DRILL_API_URL/api/v1/projects/$TASKTIPS_DRILL_PROJECT_ID/restores/$TASKTIPS_DRILL_RESTORE_ID" \
    | jq -r '.status'
}

status=$(read_status)
if [[ "$status" != "running" ]]; then
  printf 'failure drill requires a running restore (got %s)\n' "$status" >&2
  exit 1
fi

# Simulate a worker crash without touching any payload or object-store bytes.
updated_restore=$(psql "$TASKTIPS_DRILL_DATABASE_URL" -v ON_ERROR_STOP=1 -At \
  -v restore_id="$TASKTIPS_DRILL_RESTORE_ID" \
  -c "UPDATE restore_jobs SET lease_expires_at = CURRENT_TIMESTAMP - INTERVAL '1 second' WHERE id = :'restore_id' AND status = 'running' RETURNING id;" \
  | tr -d '[:space:]')
if [[ "$updated_restore" != "$TASKTIPS_DRILL_RESTORE_ID" ]]; then
  printf 'restore drill could not fence the requested running job\n' >&2
  exit 1
fi

while (( SECONDS < deadline )); do
  status=$(read_status)
  case "$status" in
    succeeded|failed|cancelled)
      printf 'restore failure drill completed with status=%s\n' "$status"
      exit 0
      ;;
    queued|running)
      sleep "$poll_seconds"
      ;;
    *)
      printf 'unexpected restore status=%s\n' "$status" >&2
      exit 1
      ;;
  esac
done

printf 'restore failure drill timed out after %ss\n' "$timeout_seconds" >&2
exit 1
