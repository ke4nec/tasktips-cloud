#!/usr/bin/env bash
set -Eeuo pipefail

# This command restores into a separately provisioned database/object store.
# It is intentionally opt-in because pg_restore replaces objects in its target.
: "${TASKTIPS_BACKUP_DRILL_CONFIRM:?set TASKTIPS_BACKUP_DRILL_CONFIRM=YES for an isolated backup drill}"
if [[ "$TASKTIPS_BACKUP_DRILL_CONFIRM" != "YES" ]]; then
  printf '%s\n' 'refusing backup drill without TASKTIPS_BACKUP_DRILL_CONFIRM=YES' >&2
  exit 2
fi
: "${TASKTIPS_DATABASE_URL:?TASKTIPS_DATABASE_URL is required}"
: "${TASKTIPS_RESTORE_DATABASE_URL:?TASKTIPS_RESTORE_DATABASE_URL is required}"
: "${TASKTIPS_RESTORE_RUSTFS_ENDPOINT:?TASKTIPS_RESTORE_RUSTFS_ENDPOINT is required}"
: "${TASKTIPS_RESTORE_RUSTFS_BUCKET:?TASKTIPS_RESTORE_RUSTFS_BUCKET is required}"
: "${TASKTIPS_RESTORE_RUSTFS_ACCESS_KEY:?TASKTIPS_RESTORE_RUSTFS_ACCESS_KEY is required}"
: "${TASKTIPS_RESTORE_RUSTFS_SECRET_KEY:?TASKTIPS_RESTORE_RUSTFS_SECRET_KEY is required}"

backup_root=${TASKTIPS_BACKUP_DRILL_DIR:-$(mktemp -d)}
cleanup() {
  if [[ -z "${TASKTIPS_BACKUP_DRILL_DIR:-}" ]]; then
    rm -rf -- "$backup_root"
  fi
}
trap cleanup EXIT

TASKTIPS_DATABASE_URL="$TASKTIPS_DATABASE_URL" \
  RUSTFS_ENDPOINT="${RUSTFS_ENDPOINT:?RUSTFS_ENDPOINT is required}" \
  RUSTFS_BUCKET="${RUSTFS_BUCKET:?RUSTFS_BUCKET is required}" \
  RUSTFS_ACCESS_KEY="${RUSTFS_ACCESS_KEY:?RUSTFS_ACCESS_KEY is required}" \
  RUSTFS_SECRET_KEY="${RUSTFS_SECRET_KEY:?RUSTFS_SECRET_KEY is required}" \
  "$(dirname "$0")/backup.sh" "$backup_root"

(cd "$backup_root" && sha256sum --check SHA256SUMS)
TASKTIPS_RESTORE_CONFIRM=YES \
  TASKTIPS_RESTORE_DATABASE_URL="$TASKTIPS_RESTORE_DATABASE_URL" \
  TASKTIPS_RESTORE_RUSTFS_ENDPOINT="$TASKTIPS_RESTORE_RUSTFS_ENDPOINT" \
  TASKTIPS_RESTORE_RUSTFS_BUCKET="$TASKTIPS_RESTORE_RUSTFS_BUCKET" \
  TASKTIPS_RESTORE_RUSTFS_ACCESS_KEY="$TASKTIPS_RESTORE_RUSTFS_ACCESS_KEY" \
  TASKTIPS_RESTORE_RUSTFS_SECRET_KEY="$TASKTIPS_RESTORE_RUSTFS_SECRET_KEY" \
  "$(dirname "$0")/restore-drill.sh" "$backup_root"

psql "$TASKTIPS_RESTORE_DATABASE_URL" -v ON_ERROR_STOP=1 \
  -c "SELECT 1 FROM instance_settings WHERE key = 'identity_schema_version';" >/dev/null
printf 'backup drill completed against isolated restore targets\n'
