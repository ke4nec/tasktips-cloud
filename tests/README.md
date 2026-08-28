# Cross-crate tests

- `contract/`: OpenAPI schema, generated-client, and compatibility checks.
- `integration/`: PostgreSQL, RLS, RustFS, and cross-storage consistency tests.
- `e2e/`: API, worker, admin, and multi-device workflows.

Unit tests remain next to the module they cover.

For an isolated environment, run `tests/e2e/sync-smoke.sh` with a fixture token, project, and payload
file. Run `deploy/backup-drill.sh` only with `TASKTIPS_BACKUP_DRILL_CONFIRM=YES` and separately
provisioned restore database/RustFS credentials; it verifies checksums, loads the dump and objects,
and confirms the restored schema before traffic is accepted.
