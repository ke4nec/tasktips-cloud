CREATE TABLE snapshots (
    id UUID PRIMARY KEY,
    project_id UUID NOT NULL,
    owner_user_id UUID NOT NULL,
    generation BIGINT NOT NULL CHECK (generation > 0),
    change_sequence BIGINT NOT NULL CHECK (change_sequence >= 0),
    manifest_hash CHAR(64) NOT NULL,
    manifest_bucket TEXT NOT NULL,
    manifest_key TEXT NOT NULL,
    manifest JSONB NOT NULL,
    status TEXT NOT NULL DEFAULT 'ready' CHECK (status IN ('pending', 'ready', 'failed')),
    created_by UUID NOT NULL REFERENCES users(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (project_id, owner_user_id) REFERENCES projects(id, owner_user_id),
    UNIQUE (id, project_id),
    UNIQUE (project_id, manifest_hash)
);

CREATE INDEX snapshots_project_created_idx ON snapshots (project_id, created_at DESC);

CREATE TABLE restore_jobs (
    id UUID PRIMARY KEY,
    project_id UUID NOT NULL,
    owner_user_id UUID NOT NULL,
    requested_by UUID NOT NULL REFERENCES users(id),
    snapshot_id UUID,
    target_change_sequence BIGINT CHECK (target_change_sequence >= 0),
    reason TEXT NOT NULL CHECK (char_length(reason) BETWEEN 1 AND 512),
    request_id TEXT,
    status TEXT NOT NULL DEFAULT 'queued'
        CHECK (status IN ('queued', 'running', 'succeeded', 'failed')),
    pre_restore_snapshot_id UUID,
    generation_before BIGINT CHECK (generation_before > 0),
    generation_after BIGINT CHECK (generation_after > 0),
    restored_objects INTEGER NOT NULL DEFAULT 0 CHECK (restored_objects >= 0),
    restored_tombstones INTEGER NOT NULL DEFAULT 0 CHECK (restored_tombstones >= 0),
    error_code TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ,
    FOREIGN KEY (project_id, owner_user_id) REFERENCES projects(id, owner_user_id),
    FOREIGN KEY (snapshot_id, project_id) REFERENCES snapshots(id, project_id),
    FOREIGN KEY (pre_restore_snapshot_id, project_id) REFERENCES snapshots(id, project_id),
    CHECK ((snapshot_id IS NOT NULL) <> (target_change_sequence IS NOT NULL))
);

CREATE INDEX restore_jobs_queue_idx ON restore_jobs (status, created_at);
CREATE INDEX restore_jobs_project_idx ON restore_jobs (project_id, created_at DESC);

ALTER TABLE snapshots ENABLE ROW LEVEL SECURITY;
ALTER TABLE snapshots FORCE ROW LEVEL SECURITY;
ALTER TABLE restore_jobs ENABLE ROW LEVEL SECURITY;
ALTER TABLE restore_jobs FORCE ROW LEVEL SECURITY;

CREATE POLICY snapshots_owner ON snapshots
    USING (owner_user_id = NULLIF(current_setting('app.user_id', true), '')::uuid)
    WITH CHECK (owner_user_id = NULLIF(current_setting('app.user_id', true), '')::uuid);
CREATE POLICY restore_jobs_owner ON restore_jobs
    USING (owner_user_id = NULLIF(current_setting('app.user_id', true), '')::uuid)
    WITH CHECK (owner_user_id = NULLIF(current_setting('app.user_id', true), '')::uuid);
CREATE POLICY snapshots_worker ON snapshots
    USING (CURRENT_USER = 'tasktips_worker')
    WITH CHECK (CURRENT_USER = 'tasktips_worker');
CREATE POLICY restore_jobs_worker ON restore_jobs
    USING (CURRENT_USER = 'tasktips_worker')
    WITH CHECK (CURRENT_USER = 'tasktips_worker');
CREATE POLICY projects_worker ON projects
    USING (CURRENT_USER = 'tasktips_worker')
    WITH CHECK (CURRENT_USER = 'tasktips_worker');
CREATE POLICY object_heads_worker ON object_heads
    USING (CURRENT_USER = 'tasktips_worker')
    WITH CHECK (CURRENT_USER = 'tasktips_worker');
CREATE POLICY change_log_worker ON change_log
    USING (CURRENT_USER = 'tasktips_worker')
    WITH CHECK (CURRENT_USER = 'tasktips_worker');

GRANT SELECT, INSERT, UPDATE ON snapshots, restore_jobs TO tasktips_app;
GRANT SELECT, INSERT, UPDATE ON snapshots, restore_jobs TO tasktips_worker;
GRANT SELECT, UPDATE ON projects TO tasktips_worker;
GRANT SELECT, INSERT ON object_revisions, change_log TO tasktips_worker;
GRANT SELECT, INSERT, UPDATE ON object_heads TO tasktips_worker;
GRANT INSERT ON audit_events TO tasktips_worker;
GRANT USAGE, SELECT ON SEQUENCE audit_events_id_seq TO tasktips_worker;
GRANT USAGE ON SCHEMA public TO tasktips_worker;

INSERT INTO instance_settings (key, value)
VALUES ('history_schema_version', '{"version": 1}'::jsonb)
ON CONFLICT (key) DO NOTHING;
