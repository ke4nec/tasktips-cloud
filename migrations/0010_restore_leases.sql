ALTER TABLE restore_jobs
    ADD COLUMN lease_token UUID,
    ADD COLUMN lease_expires_at TIMESTAMPTZ;

CREATE INDEX restore_jobs_lease_idx
    ON restore_jobs (status, lease_expires_at)
    WHERE status = 'running';

INSERT INTO instance_settings (key, value)
VALUES ('restore_lease_schema_version', '{"version": 1}'::jsonb)
ON CONFLICT (key) DO NOTHING;
