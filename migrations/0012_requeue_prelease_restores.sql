-- Any pre-lease worker is gone after migration downtime; make those jobs reclaimable.
UPDATE restore_jobs
SET status = 'queued', started_at = NULL, finished_at = NULL
WHERE status = 'running' AND lease_token IS NULL;

INSERT INTO instance_settings (key, value)
VALUES ('restore_requeue_schema_version', '{"version": 1}'::jsonb)
ON CONFLICT (key) DO NOTHING;
