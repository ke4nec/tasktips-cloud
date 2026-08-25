-- Keep system-admin access limited to operational metadata and aggregate counts.
CREATE VIEW admin_operational_counts AS
SELECT
    (SELECT count(*) FROM admin_user_metadata)::bigint AS users,
    (SELECT count(*) FROM admin_user_metadata WHERE status = 'active')::bigint AS active_users,
    (SELECT count(*) FROM admin_project_metadata)::bigint AS projects,
    (SELECT count(*) FROM admin_device_metadata)::bigint AS devices,
    (SELECT count(*) FROM object_revisions)::bigint AS revisions,
    (SELECT count(*) FROM object_revisions WHERE is_tombstone)::bigint AS tombstones,
    (SELECT COALESCE(sum(size_bytes), 0)::bigint FROM payloads) AS payload_bytes,
    (SELECT count(*) FROM restore_jobs WHERE status IN ('queued', 'running'))::bigint AS queued_restores;

CREATE VIEW admin_history_metadata AS
SELECT c.project_id,
       c.sequence AS change_sequence,
       r.kind::text AS kind,
       r.object_id,
       r.revision,
       r.base_revision,
       r.changed_at,
       r.device_id,
       r.is_tombstone AS tombstone
FROM change_log AS c
JOIN object_revisions AS r ON r.id = c.revision_id;

CREATE VIEW admin_snapshot_metadata AS
SELECT id, project_id, status
FROM snapshots;

CREATE VIEW admin_restore_job_metadata AS
SELECT id, project_id, requested_by, snapshot_id, target_change_sequence, reason,
       status, pre_restore_snapshot_id, generation_before, generation_after,
       restored_objects, restored_tombstones, error_code, created_at, started_at, finished_at
FROM restore_jobs;

REVOKE SELECT ON object_revisions, change_log, snapshots, payloads, restore_jobs FROM tasktips_admin;
GRANT INSERT ON restore_jobs TO tasktips_admin;
GRANT SELECT ON admin_operational_counts, admin_history_metadata,
    admin_snapshot_metadata, admin_restore_job_metadata TO tasktips_admin;

INSERT INTO instance_settings (key, value)
VALUES ('admin_safe_views_schema_version', '{"version": 1}'::jsonb)
ON CONFLICT (key) DO NOTHING;
