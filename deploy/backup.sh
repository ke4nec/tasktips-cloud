#!/usr/bin/env sh
set -eu

backup_root=${1:?usage: deploy/backup.sh BACKUP_DIRECTORY}
: "${TASKTIPS_DATABASE_URL:?TASKTIPS_DATABASE_URL is required}"
: "${RUSTFS_ENDPOINT:?RUSTFS_ENDPOINT is required}"
: "${RUSTFS_BUCKET:?RUSTFS_BUCKET is required}"
: "${RUSTFS_ACCESS_KEY:?RUSTFS_ACCESS_KEY is required}"
: "${RUSTFS_SECRET_KEY:?RUSTFS_SECRET_KEY is required}"

mkdir -p "$backup_root/postgres" "$backup_root/rustfs"
pg_dump --format=custom --no-owner --file "$backup_root/postgres/tasktips.dump" "$TASKTIPS_DATABASE_URL"

mc alias set tasktips-backup "$RUSTFS_ENDPOINT" "$RUSTFS_ACCESS_KEY" "$RUSTFS_SECRET_KEY" >/dev/null
mc mirror --overwrite "tasktips-backup/$RUSTFS_BUCKET" "$backup_root/rustfs"

sha256sum "$backup_root/postgres/tasktips.dump" > "$backup_root/SHA256SUMS"
find "$backup_root/rustfs" -type f -print0 | sort -z | xargs -0 -r sha256sum >> "$backup_root/SHA256SUMS"
date -u +%Y-%m-%dT%H:%M:%SZ > "$backup_root/created-at"
printf '%s\n' "backup complete: $backup_root"
