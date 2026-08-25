CREATE EXTENSION IF NOT EXISTS pgcrypto;

CREATE TYPE user_role AS ENUM ('user', 'system_admin');
CREATE TYPE account_status AS ENUM ('active', 'disabled', 'pending', 'deleting');
CREATE TYPE project_status AS ENUM ('active', 'maintenance', 'disabled', 'deleting');

CREATE TABLE users (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    email_normalized TEXT NOT NULL UNIQUE,
    email_display TEXT NOT NULL,
    password_hash TEXT NOT NULL,
    role user_role NOT NULL DEFAULT 'user',
    status account_status NOT NULL DEFAULT 'pending',
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    last_login_at TIMESTAMPTZ
);

CREATE TABLE invitations (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    email_normalized TEXT NOT NULL UNIQUE,
    email_display TEXT NOT NULL,
    token_hash BYTEA NOT NULL UNIQUE,
    role user_role NOT NULL DEFAULT 'user',
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    used_at TIMESTAMPTZ
);

CREATE TABLE projects (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    owner_user_id UUID NOT NULL REFERENCES users(id),
    name TEXT NOT NULL,
    generation BIGINT NOT NULL DEFAULT 1 CHECK (generation > 0),
    status project_status NOT NULL DEFAULT 'active',
    change_seq BIGINT NOT NULL DEFAULT 0 CHECK (change_seq >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE (id, owner_user_id)
);

CREATE TABLE devices (
    id UUID PRIMARY KEY,
    owner_user_id UUID NOT NULL REFERENCES users(id),
    display_name TEXT NOT NULL,
    platform TEXT NOT NULL,
    app_version TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    last_seen_at TIMESTAMPTZ,
    last_login_at TIMESTAMPTZ,
    last_pull_at TIMESTAMPTZ,
    last_push_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ,
    UNIQUE (id, owner_user_id)
);

CREATE TABLE refresh_tokens (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id),
    device_id UUID NOT NULL,
    token_hash BYTEA NOT NULL UNIQUE,
    family_id UUID NOT NULL,
    issued_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    expires_at TIMESTAMPTZ NOT NULL,
    used_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ,
    FOREIGN KEY (device_id, user_id) REFERENCES devices(id, owner_user_id)
);

CREATE INDEX refresh_tokens_family_idx ON refresh_tokens (family_id);
CREATE INDEX projects_owner_idx ON projects (owner_user_id);
CREATE INDEX devices_owner_idx ON devices (owner_user_id);

CREATE TABLE audit_events (
    id BIGSERIAL PRIMARY KEY,
    actor_user_id UUID REFERENCES users(id),
    subject_user_id UUID REFERENCES users(id),
    project_id UUID REFERENCES projects(id),
    action TEXT NOT NULL,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    request_id TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE VIEW admin_user_metadata AS
SELECT id, email_normalized, email_display, role, status, created_at, last_login_at
FROM users;

CREATE VIEW admin_project_metadata AS
SELECT p.id, p.owner_user_id, p.name, p.generation, p.status, p.change_seq, p.created_at, p.updated_at
FROM projects AS p;

CREATE VIEW admin_device_metadata AS
SELECT id, owner_user_id, display_name, platform, app_version, created_at,
       last_seen_at, last_login_at, last_pull_at, last_push_at, revoked_at
FROM devices;

ALTER TABLE users ENABLE ROW LEVEL SECURITY;
ALTER TABLE users FORCE ROW LEVEL SECURITY;
ALTER TABLE invitations ENABLE ROW LEVEL SECURITY;
ALTER TABLE invitations FORCE ROW LEVEL SECURITY;
ALTER TABLE projects ENABLE ROW LEVEL SECURITY;
ALTER TABLE projects FORCE ROW LEVEL SECURITY;
ALTER TABLE devices ENABLE ROW LEVEL SECURITY;
ALTER TABLE devices FORCE ROW LEVEL SECURITY;
ALTER TABLE refresh_tokens ENABLE ROW LEVEL SECURITY;
ALTER TABLE refresh_tokens FORCE ROW LEVEL SECURITY;
ALTER TABLE audit_events ENABLE ROW LEVEL SECURITY;
ALTER TABLE audit_events FORCE ROW LEVEL SECURITY;

CREATE POLICY users_self ON users
    USING (id = NULLIF(current_setting('app.user_id', true), '')::uuid)
    WITH CHECK (id = NULLIF(current_setting('app.user_id', true), '')::uuid);
CREATE POLICY users_internal ON users
    USING (CURRENT_USER = 'tasktips_auth')
    WITH CHECK (CURRENT_USER = 'tasktips_auth');
CREATE POLICY invitations_admin ON invitations
    USING (current_setting('app.role', true) = 'system_admin')
    WITH CHECK (current_setting('app.role', true) = 'system_admin');
CREATE POLICY invitations_internal ON invitations
    USING (CURRENT_USER = 'tasktips_auth')
    WITH CHECK (CURRENT_USER = 'tasktips_auth');
CREATE POLICY projects_owner ON projects
    USING (owner_user_id = NULLIF(current_setting('app.user_id', true), '')::uuid)
    WITH CHECK (owner_user_id = NULLIF(current_setting('app.user_id', true), '')::uuid);
CREATE POLICY devices_owner ON devices
    USING (owner_user_id = NULLIF(current_setting('app.user_id', true), '')::uuid)
    WITH CHECK (owner_user_id = NULLIF(current_setting('app.user_id', true), '')::uuid);
CREATE POLICY devices_internal ON devices
    USING (CURRENT_USER = 'tasktips_auth')
    WITH CHECK (CURRENT_USER = 'tasktips_auth');
CREATE POLICY refresh_tokens_owner ON refresh_tokens
    USING (user_id = NULLIF(current_setting('app.user_id', true), '')::uuid)
    WITH CHECK (user_id = NULLIF(current_setting('app.user_id', true), '')::uuid);
CREATE POLICY refresh_tokens_internal ON refresh_tokens
    USING (CURRENT_USER = 'tasktips_auth')
    WITH CHECK (CURRENT_USER = 'tasktips_auth');
CREATE POLICY audit_events_admin ON audit_events
    USING (current_setting('app.role', true) = 'system_admin')
    WITH CHECK (current_setting('app.role', true) = 'system_admin');
CREATE POLICY audit_events_internal ON audit_events
    USING (CURRENT_USER = 'tasktips_auth')
    WITH CHECK (CURRENT_USER = 'tasktips_auth');

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'tasktips_auth') THEN
        CREATE ROLE tasktips_auth NOLOGIN;
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'tasktips_app') THEN
        CREATE ROLE tasktips_app NOLOGIN;
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'tasktips_admin') THEN
        CREATE ROLE tasktips_admin NOLOGIN;
    END IF;
END $$;

GRANT tasktips_auth, tasktips_app, tasktips_admin TO CURRENT_USER;
GRANT USAGE ON SCHEMA public TO tasktips_auth, tasktips_app, tasktips_admin;
GRANT SELECT, INSERT, UPDATE ON users, invitations, devices, refresh_tokens, audit_events TO tasktips_auth;
GRANT USAGE, SELECT ON SEQUENCE audit_events_id_seq TO tasktips_auth;
GRANT SELECT, INSERT, UPDATE ON users, projects, devices, refresh_tokens TO tasktips_app;
GRANT SELECT, INSERT ON invitations, audit_events TO tasktips_admin;
GRANT SELECT ON admin_user_metadata, admin_project_metadata, admin_device_metadata TO tasktips_admin;
GRANT USAGE, SELECT ON SEQUENCE audit_events_id_seq TO tasktips_admin;

INSERT INTO instance_settings (key, value)
VALUES ('identity_schema_version', '{"version": 1}'::jsonb)
ON CONFLICT (key) DO NOTHING;
