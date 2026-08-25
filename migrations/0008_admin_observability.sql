CREATE TABLE sync_attempts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    owner_user_id UUID NOT NULL REFERENCES users(id),
    project_id UUID NOT NULL,
    device_id UUID,
    operation TEXT NOT NULL CHECK (operation IN ('bootstrap', 'pull', 'push')),
    status TEXT NOT NULL CHECK (status IN ('succeeded', 'conflict', 'rejected', 'failed')),
    error_code TEXT,
    item_count INTEGER NOT NULL DEFAULT 0 CHECK (item_count >= 0),
    latency_ms INTEGER CHECK (latency_ms >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (project_id, owner_user_id) REFERENCES projects(id, owner_user_id),
    FOREIGN KEY (device_id, owner_user_id) REFERENCES devices(id, owner_user_id)
);

CREATE INDEX sync_attempts_created_idx ON sync_attempts (created_at DESC);
CREATE INDEX sync_attempts_project_idx ON sync_attempts (project_id, created_at DESC);

ALTER TABLE sync_attempts ENABLE ROW LEVEL SECURITY;
ALTER TABLE sync_attempts FORCE ROW LEVEL SECURITY;

CREATE POLICY sync_attempts_owner ON sync_attempts
    USING (owner_user_id = NULLIF(current_setting('app.user_id', true), '')::uuid)
    WITH CHECK (owner_user_id = NULLIF(current_setting('app.user_id', true), '')::uuid);
CREATE POLICY sync_attempts_admin ON sync_attempts
    USING (CURRENT_USER = 'tasktips_admin')
    WITH CHECK (CURRENT_USER = 'tasktips_admin');

CREATE POLICY audit_events_admin_role ON audit_events
    USING (CURRENT_USER = 'tasktips_admin')
    WITH CHECK (CURRENT_USER = 'tasktips_admin');
CREATE POLICY object_revisions_admin ON object_revisions
    USING (CURRENT_USER = 'tasktips_admin')
    WITH CHECK (CURRENT_USER = 'tasktips_admin');
CREATE POLICY change_log_admin ON change_log
    USING (CURRENT_USER = 'tasktips_admin')
    WITH CHECK (CURRENT_USER = 'tasktips_admin');
CREATE POLICY restore_jobs_admin ON restore_jobs
    USING (CURRENT_USER = 'tasktips_admin')
    WITH CHECK (CURRENT_USER = 'tasktips_admin');
CREATE POLICY snapshots_admin ON snapshots
    USING (CURRENT_USER = 'tasktips_admin')
    WITH CHECK (CURRENT_USER = 'tasktips_admin');
CREATE POLICY payloads_admin ON payloads
    USING (CURRENT_USER = 'tasktips_admin')
    WITH CHECK (CURRENT_USER = 'tasktips_admin');

GRANT SELECT, INSERT ON sync_attempts TO tasktips_app;
GRANT SELECT ON sync_attempts, audit_events, object_revisions, change_log, restore_jobs TO tasktips_admin;
GRANT SELECT ON snapshots TO tasktips_admin;
GRANT SELECT ON payloads TO tasktips_admin;

INSERT INTO instance_settings (key, value)
VALUES ('admin_schema_version', '{"version": 1}'::jsonb)
ON CONFLICT (key) DO NOTHING;
