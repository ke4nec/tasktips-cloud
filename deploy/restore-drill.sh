#!/usr/bin/env sh
set -eu

backup_root=${1:?usage: deploy/restore-drill.sh BACKUP_DIRECTORY}
: "${TASKTIPS_RESTORE_DATABASE_URL:?TASKTIPS_RESTORE_DATABASE_URL is required}"
: "${TASKTIPS_RESTORE_RUSTFS_ENDPOINT:?TASKTIPS_RESTORE_RUSTFS_ENDPOINT is required}"
: "${TASKTIPS_RESTORE_RUSTFS_BUCKET:?TASKTIPS_RESTORE_RUSTFS_BUCKET is required}"
: "${TASKTIPS_RESTORE_RUSTFS_ACCESS_KEY:?TASKTIPS_RESTORE_RUSTFS_ACCESS_KEY is required}"
: "${TASKTIPS_RESTORE_RUSTFS_SECRET_KEY:?TASKTIPS_RESTORE_RUSTFS_SECRET_KEY is required}"
: "${TASKTIPS_RESTORE_CONFIRM:?set TASKTIPS_RESTORE_CONFIRM=YES for an isolated restore drill}"
[ "$TASKTIPS_RESTORE_CONFIRM" = YES ]

(cd "$backup_root" && sha256sum --check SHA256SUMS)
pg_restore --clean --if-exists --no-owner --dbname "$TASKTIPS_RESTORE_DATABASE_URL" \
  "$backup_root/postgres/tasktips.dump"

mc alias set tasktips-restore "$TASKTIPS_RESTORE_RUSTFS_ENDPOINT" \
  "$TASKTIPS_RESTORE_RUSTFS_ACCESS_KEY" "$TASKTIPS_RESTORE_RUSTFS_SECRET_KEY" >/dev/null
mc mb --ignore-existing "tasktips-restore/$TASKTIPS_RESTORE_RUSTFS_BUCKET" >/dev/null
mc mirror --overwrite "$backup_root/rustfs" "tasktips-restore/$TASKTIPS_RESTORE_RUSTFS_BUCKET"

printf '%s\n' "restore drill data loaded; start the restored API and run bootstrap, payload, history, snapshot, and restore checks before accepting traffic"
