CREATE TYPE sync_object_kind AS ENUM ('todo', 'classification', 'index', 'image');

CREATE TABLE payloads (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id UUID NOT NULL,
    owner_user_id UUID NOT NULL,
    content_hash CHAR(64) NOT NULL,
    bucket TEXT NOT NULL,
    object_key TEXT NOT NULL,
    size_bytes BIGINT NOT NULL CHECK (size_bytes >= 0),
    media_type TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE (project_id, content_hash),
    UNIQUE (id, project_id),
    FOREIGN KEY (project_id, owner_user_id) REFERENCES projects(id, owner_user_id)
);

CREATE TABLE object_revisions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id UUID NOT NULL,
    owner_user_id UUID NOT NULL,
    kind sync_object_kind NOT NULL,
    object_id TEXT NOT NULL,
    revision BIGINT NOT NULL CHECK (revision > 0),
    base_revision BIGINT CHECK (base_revision > 0),
    schema_version INTEGER CHECK (schema_version > 0),
    payload_id UUID,
    content_hash CHAR(64),
    is_tombstone BOOLEAN NOT NULL,
    changed_at TIMESTAMPTZ NOT NULL,
    device_id UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE (project_id, kind, object_id, revision),
    UNIQUE (id, project_id),
    FOREIGN KEY (project_id, owner_user_id) REFERENCES projects(id, owner_user_id),
    FOREIGN KEY (payload_id, project_id) REFERENCES payloads(id, project_id),
    FOREIGN KEY (device_id, owner_user_id) REFERENCES devices(id, owner_user_id),
    CHECK (
        (is_tombstone AND payload_id IS NULL AND content_hash IS NULL AND schema_version IS NULL)
        OR
        (NOT is_tombstone AND payload_id IS NOT NULL AND content_hash IS NOT NULL AND schema_version IS NOT NULL)
    )
);

CREATE TABLE object_heads (
    project_id UUID NOT NULL,
    owner_user_id UUID NOT NULL,
    kind sync_object_kind NOT NULL,
    object_id TEXT NOT NULL,
    revision BIGINT NOT NULL CHECK (revision > 0),
    revision_id UUID NOT NULL,
    PRIMARY KEY (project_id, kind, object_id),
    FOREIGN KEY (project_id, owner_user_id) REFERENCES projects(id, owner_user_id),
    FOREIGN KEY (revision_id, project_id) REFERENCES object_revisions(id, project_id)
);

CREATE TABLE change_log (
    project_id UUID NOT NULL,
    owner_user_id UUID NOT NULL,
    sequence BIGINT NOT NULL CHECK (sequence > 0),
    revision_id UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (project_id, sequence),
    UNIQUE (revision_id),
    FOREIGN KEY (project_id, owner_user_id) REFERENCES projects(id, owner_user_id),
    FOREIGN KEY (revision_id, project_id) REFERENCES object_revisions(id, project_id)
);

CREATE TABLE bootstrap_completions (
    project_id UUID NOT NULL,
    owner_user_id UUID NOT NULL,
    device_id UUID NOT NULL,
    generation BIGINT NOT NULL CHECK (generation > 0),
    change_sequence BIGINT NOT NULL CHECK (change_sequence >= 0),
    completed_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (project_id, device_id, generation),
    FOREIGN KEY (project_id, owner_user_id) REFERENCES projects(id, owner_user_id),
    FOREIGN KEY (device_id, owner_user_id) REFERENCES devices(id, owner_user_id)
);

CREATE TABLE idempotency_records (
    owner_user_id UUID NOT NULL REFERENCES users(id),
    device_id UUID NOT NULL,
    project_id UUID NOT NULL,
    request_key TEXT NOT NULL,
    request_hash CHAR(64) NOT NULL,
    response JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (owner_user_id, device_id, request_key),
    FOREIGN KEY (device_id, owner_user_id) REFERENCES devices(id, owner_user_id),
    FOREIGN KEY (project_id, owner_user_id) REFERENCES projects(id, owner_user_id)
);

CREATE INDEX object_revisions_history_idx
    ON object_revisions (project_id, kind, object_id, revision DESC);
CREATE INDEX change_log_sequence_idx ON change_log (project_id, sequence);
CREATE INDEX payloads_created_idx ON payloads (created_at);

ALTER TABLE payloads ENABLE ROW LEVEL SECURITY;
ALTER TABLE payloads FORCE ROW LEVEL SECURITY;
ALTER TABLE object_revisions ENABLE ROW LEVEL SECURITY;
ALTER TABLE object_revisions FORCE ROW LEVEL SECURITY;
ALTER TABLE object_heads ENABLE ROW LEVEL SECURITY;
ALTER TABLE object_heads FORCE ROW LEVEL SECURITY;
ALTER TABLE change_log ENABLE ROW LEVEL SECURITY;
ALTER TABLE change_log FORCE ROW LEVEL SECURITY;
ALTER TABLE bootstrap_completions ENABLE ROW LEVEL SECURITY;
ALTER TABLE bootstrap_completions FORCE ROW LEVEL SECURITY;
ALTER TABLE idempotency_records ENABLE ROW LEVEL SECURITY;
ALTER TABLE idempotency_records FORCE ROW LEVEL SECURITY;

CREATE POLICY payloads_owner ON payloads
    USING (owner_user_id = NULLIF(current_setting('app.user_id', true), '')::uuid)
    WITH CHECK (owner_user_id = NULLIF(current_setting('app.user_id', true), '')::uuid);
CREATE POLICY object_revisions_owner ON object_revisions
    USING (owner_user_id = NULLIF(current_setting('app.user_id', true), '')::uuid)
    WITH CHECK (owner_user_id = NULLIF(current_setting('app.user_id', true), '')::uuid);
CREATE POLICY object_heads_owner ON object_heads
    USING (owner_user_id = NULLIF(current_setting('app.user_id', true), '')::uuid)
    WITH CHECK (owner_user_id = NULLIF(current_setting('app.user_id', true), '')::uuid);
CREATE POLICY change_log_owner ON change_log
    USING (owner_user_id = NULLIF(current_setting('app.user_id', true), '')::uuid)
    WITH CHECK (owner_user_id = NULLIF(current_setting('app.user_id', true), '')::uuid);
CREATE POLICY bootstrap_completions_owner ON bootstrap_completions
    USING (owner_user_id = NULLIF(current_setting('app.user_id', true), '')::uuid)
    WITH CHECK (owner_user_id = NULLIF(current_setting('app.user_id', true), '')::uuid);
CREATE POLICY idempotency_records_owner ON idempotency_records
    USING (owner_user_id = NULLIF(current_setting('app.user_id', true), '')::uuid)
    WITH CHECK (owner_user_id = NULLIF(current_setting('app.user_id', true), '')::uuid);

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'tasktips_worker') THEN
        CREATE ROLE tasktips_worker NOLOGIN;
    END IF;
END $$;

GRANT tasktips_worker TO CURRENT_USER;
GRANT USAGE ON SCHEMA public TO tasktips_worker;
CREATE POLICY payloads_worker ON payloads
    USING (CURRENT_USER = 'tasktips_worker')
    WITH CHECK (CURRENT_USER = 'tasktips_worker');
CREATE POLICY object_revisions_worker ON object_revisions
    USING (CURRENT_USER = 'tasktips_worker')
    WITH CHECK (CURRENT_USER = 'tasktips_worker');
GRANT SELECT, DELETE ON payloads TO tasktips_worker;
GRANT SELECT ON object_revisions TO tasktips_worker;

GRANT SELECT, INSERT ON payloads TO tasktips_app;
GRANT SELECT, INSERT ON object_revisions, change_log, bootstrap_completions, idempotency_records TO tasktips_app;
GRANT SELECT, INSERT, UPDATE ON object_heads TO tasktips_app;
GRANT SELECT, UPDATE ON projects, devices TO tasktips_app;

INSERT INTO instance_settings (key, value)
VALUES ('sync_schema_version', '{"version": 1}'::jsonb)
ON CONFLICT (key) DO NOTHING;
