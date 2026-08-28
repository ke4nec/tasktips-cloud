-- TaskTips Cloud initial PostgreSQL schema.
-- This is the single source used for a new database; SQLx creates its own migration table.

CREATE EXTENSION IF NOT EXISTS pgcrypto WITH SCHEMA public;

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
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'tasktips_worker') THEN
        CREATE ROLE tasktips_worker NOLOGIN;
    END IF;
END $$;

GRANT tasktips_auth, tasktips_app, tasktips_admin, tasktips_worker TO CURRENT_USER;

CREATE TYPE public.account_status AS ENUM (
    'active',
    'disabled',
    'pending',
    'deleting',
    'deleted'
);

CREATE TYPE public.project_status AS ENUM (
    'active',
    'maintenance',
    'disabled',
    'deleting'
);

CREATE TYPE public.sync_object_kind AS ENUM (
    'todo',
    'classification',
    'index',
    'image'
);

CREATE TYPE public.user_role AS ENUM (
    'user',
    'system_admin'
);

-- These functions reference tables declared below; defer body validation until
-- the complete schema has been created.
SET check_function_bodies = false;

CREATE FUNCTION public.admin_lock_project(target_project_id uuid) RETURNS TABLE(owner_user_id uuid, change_seq bigint, status public.project_status)
    LANGUAGE sql SECURITY DEFINER
    SET search_path TO 'public'
    AS $$
    SELECT p.owner_user_id, p.change_seq, p.status
    FROM projects AS p
    WHERE p.id = target_project_id
    FOR UPDATE
$$;

CREATE FUNCTION public.app_current_user_id() RETURNS uuid
    LANGUAGE plpgsql STABLE
    SET search_path TO 'pg_catalog', 'public'
    AS $$
BEGIN
    RETURN NULLIF(current_setting('app.user_id', true), '')::uuid;
EXCEPTION
    WHEN invalid_text_representation THEN
        RETURN NULL;
END;
$$;

SET check_function_bodies = true;

CREATE TABLE public.account_purge_exports (
    id uuid NOT NULL,
    user_id uuid NOT NULL,
    job_id uuid NOT NULL,
    object_key text NOT NULL,
    content_hash character(64),
    size_bytes bigint,
    status text DEFAULT 'pending'::text NOT NULL,
    created_at timestamp with time zone DEFAULT CURRENT_TIMESTAMP NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    completed_at timestamp with time zone,
    CONSTRAINT account_purge_exports_size_bytes_check CHECK (((size_bytes IS NULL) OR (size_bytes >= 0))),
    CONSTRAINT account_purge_exports_status_check CHECK ((status = ANY (ARRAY['pending'::text, 'ready'::text, 'purging'::text, 'completed'::text, 'failed'::text, 'expired'::text])))
);

ALTER TABLE ONLY public.account_purge_exports FORCE ROW LEVEL SECURITY;

CREATE VIEW public.admin_account_purge_export_metadata AS
 SELECT id,
    user_id,
    job_id,
    status,
    created_at,
    expires_at,
    completed_at
   FROM public.account_purge_exports;

CREATE TABLE public.devices (
    id uuid NOT NULL,
    owner_user_id uuid NOT NULL,
    display_name text NOT NULL,
    platform text NOT NULL,
    app_version text NOT NULL,
    created_at timestamp with time zone DEFAULT CURRENT_TIMESTAMP NOT NULL,
    last_seen_at timestamp with time zone,
    last_login_at timestamp with time zone,
    last_pull_at timestamp with time zone,
    last_push_at timestamp with time zone,
    revoked_at timestamp with time zone
);

ALTER TABLE ONLY public.devices FORCE ROW LEVEL SECURITY;

CREATE VIEW public.admin_device_metadata AS
 SELECT id,
    owner_user_id,
    display_name,
    platform,
    app_version,
    created_at,
    last_seen_at,
    last_login_at,
    last_pull_at,
    last_push_at,
    revoked_at
   FROM public.devices;

CREATE TABLE public.change_log (
    project_id uuid NOT NULL,
    owner_user_id uuid NOT NULL,
    sequence bigint NOT NULL,
    revision_id uuid NOT NULL,
    created_at timestamp with time zone DEFAULT CURRENT_TIMESTAMP NOT NULL,
    CONSTRAINT change_log_sequence_check CHECK ((sequence > 0))
);

ALTER TABLE ONLY public.change_log FORCE ROW LEVEL SECURITY;

CREATE TABLE public.object_revisions (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    project_id uuid NOT NULL,
    owner_user_id uuid NOT NULL,
    kind public.sync_object_kind NOT NULL,
    object_id text NOT NULL,
    revision bigint NOT NULL,
    base_revision bigint,
    schema_version integer,
    payload_id uuid,
    content_hash character(64),
    is_tombstone boolean NOT NULL,
    changed_at timestamp with time zone NOT NULL,
    device_id uuid NOT NULL,
    created_at timestamp with time zone DEFAULT CURRENT_TIMESTAMP NOT NULL,
    CONSTRAINT object_revisions_base_revision_check CHECK ((base_revision > 0)),
    CONSTRAINT object_revisions_check CHECK (((is_tombstone AND (payload_id IS NULL) AND (content_hash IS NULL) AND (schema_version IS NULL)) OR ((NOT is_tombstone) AND (payload_id IS NOT NULL) AND (content_hash IS NOT NULL) AND (schema_version IS NOT NULL)))),
    CONSTRAINT object_revisions_revision_check CHECK ((revision > 0)),
    CONSTRAINT object_revisions_schema_version_check CHECK ((schema_version > 0))
);

ALTER TABLE ONLY public.object_revisions FORCE ROW LEVEL SECURITY;

CREATE VIEW public.admin_history_metadata AS
 SELECT c.project_id,
    c.sequence AS change_sequence,
    (r.kind)::text AS kind,
    r.object_id,
    r.revision,
    r.base_revision,
    r.changed_at,
    r.device_id,
    r.is_tombstone AS tombstone
   FROM (public.change_log c
     JOIN public.object_revisions r ON ((r.id = c.revision_id)));

CREATE TABLE public.jobs (
    id uuid NOT NULL,
    kind text NOT NULL,
    owner_user_id uuid,
    project_id uuid,
    payload jsonb DEFAULT '{}'::jsonb NOT NULL,
    status text DEFAULT 'queued'::text NOT NULL,
    attempts integer DEFAULT 0 NOT NULL,
    run_after timestamp with time zone DEFAULT CURRENT_TIMESTAMP NOT NULL,
    lease_token uuid,
    lease_expires_at timestamp with time zone,
    error_code text,
    created_at timestamp with time zone DEFAULT CURRENT_TIMESTAMP NOT NULL,
    started_at timestamp with time zone,
    finished_at timestamp with time zone,
    CONSTRAINT jobs_attempts_check CHECK ((attempts >= 0)),
    CONSTRAINT jobs_kind_check CHECK ((kind = ANY (ARRAY['project_purge'::text, 'account_purge'::text, 'snapshot_cleanup'::text, 'statistics'::text]))),
    CONSTRAINT jobs_status_check CHECK ((status = ANY (ARRAY['queued'::text, 'running'::text, 'succeeded'::text, 'failed'::text])))
);

ALTER TABLE ONLY public.jobs FORCE ROW LEVEL SECURITY;

CREATE VIEW public.admin_job_metadata AS
 SELECT id,
    kind,
    owner_user_id,
    project_id,
    status,
    attempts,
    run_after,
    error_code,
    created_at,
    started_at,
    finished_at
   FROM public.jobs;

CREATE TABLE public.projects (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    owner_user_id uuid NOT NULL,
    name text NOT NULL,
    generation bigint DEFAULT 1 NOT NULL,
    status public.project_status DEFAULT 'active'::public.project_status NOT NULL,
    change_seq bigint DEFAULT 0 NOT NULL,
    created_at timestamp with time zone DEFAULT CURRENT_TIMESTAMP NOT NULL,
    updated_at timestamp with time zone DEFAULT CURRENT_TIMESTAMP NOT NULL,
    CONSTRAINT projects_change_seq_check CHECK ((change_seq >= 0)),
    CONSTRAINT projects_generation_check CHECK ((generation > 0))
);

ALTER TABLE ONLY public.projects FORCE ROW LEVEL SECURITY;

CREATE VIEW public.admin_project_metadata AS
 SELECT id,
    owner_user_id,
    name,
    generation,
    status,
    change_seq,
    created_at,
    updated_at
   FROM public.projects p;

CREATE TABLE public.users (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    email_normalized text NOT NULL,
    email_display text NOT NULL,
    password_hash text NOT NULL,
    role public.user_role DEFAULT 'user'::public.user_role NOT NULL,
    status public.account_status DEFAULT 'pending'::public.account_status NOT NULL,
    created_at timestamp with time zone DEFAULT CURRENT_TIMESTAMP NOT NULL,
    last_login_at timestamp with time zone
);

ALTER TABLE ONLY public.users FORCE ROW LEVEL SECURITY;

CREATE VIEW public.admin_user_metadata AS
 SELECT id,
    email_normalized,
    email_display,
    role,
    status,
    created_at,
    last_login_at
   FROM public.users;

CREATE TABLE public.payloads (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    project_id uuid NOT NULL,
    owner_user_id uuid NOT NULL,
    content_hash character(64) NOT NULL,
    bucket text NOT NULL,
    object_key text NOT NULL,
    size_bytes bigint NOT NULL,
    media_type text NOT NULL,
    created_at timestamp with time zone DEFAULT CURRENT_TIMESTAMP NOT NULL,
    CONSTRAINT payloads_size_bytes_check CHECK ((size_bytes >= 0))
);

ALTER TABLE ONLY public.payloads FORCE ROW LEVEL SECURITY;

CREATE TABLE public.restore_jobs (
    id uuid NOT NULL,
    project_id uuid NOT NULL,
    owner_user_id uuid NOT NULL,
    requested_by uuid NOT NULL,
    snapshot_id uuid,
    target_change_sequence bigint,
    reason text NOT NULL,
    request_id text,
    status text DEFAULT 'queued'::text NOT NULL,
    pre_restore_snapshot_id uuid,
    generation_before bigint,
    generation_after bigint,
    restored_objects integer DEFAULT 0 NOT NULL,
    restored_tombstones integer DEFAULT 0 NOT NULL,
    error_code text,
    created_at timestamp with time zone DEFAULT CURRENT_TIMESTAMP NOT NULL,
    started_at timestamp with time zone,
    finished_at timestamp with time zone,
    lease_token uuid,
    lease_expires_at timestamp with time zone,
    attempts integer DEFAULT 0 NOT NULL,
    run_after timestamp with time zone DEFAULT CURRENT_TIMESTAMP NOT NULL,
    cancel_requested boolean DEFAULT false NOT NULL,
    CONSTRAINT restore_jobs_attempts_check CHECK ((attempts >= 0)),
    CONSTRAINT restore_jobs_check CHECK (((snapshot_id IS NOT NULL) <> (target_change_sequence IS NOT NULL))),
    CONSTRAINT restore_jobs_generation_after_check CHECK ((generation_after > 0)),
    CONSTRAINT restore_jobs_generation_before_check CHECK ((generation_before > 0)),
    CONSTRAINT restore_jobs_reason_check CHECK (((char_length(reason) >= 1) AND (char_length(reason) <= 512))),
    CONSTRAINT restore_jobs_restored_objects_check CHECK ((restored_objects >= 0)),
    CONSTRAINT restore_jobs_restored_tombstones_check CHECK ((restored_tombstones >= 0)),
    CONSTRAINT restore_jobs_status_check CHECK ((status = ANY (ARRAY['queued'::text, 'running'::text, 'succeeded'::text, 'failed'::text, 'cancelled'::text]))),
    CONSTRAINT restore_jobs_target_change_sequence_check CHECK ((target_change_sequence >= 0))
);

ALTER TABLE ONLY public.restore_jobs FORCE ROW LEVEL SECURITY;

CREATE VIEW public.admin_operational_counts AS
 SELECT ( SELECT count(*) AS count
           FROM public.admin_user_metadata) AS users,
    ( SELECT count(*) AS count
           FROM public.admin_user_metadata
          WHERE (admin_user_metadata.status = 'active'::public.account_status)) AS active_users,
    ( SELECT count(*) AS count
           FROM public.admin_project_metadata) AS projects,
    ( SELECT count(*) AS count
           FROM public.admin_device_metadata) AS devices,
    ( SELECT count(*) AS count
           FROM public.object_revisions) AS revisions,
    ( SELECT count(*) AS count
           FROM public.object_revisions
          WHERE object_revisions.is_tombstone) AS tombstones,
    ( SELECT (COALESCE(sum(payloads.size_bytes), (0)::numeric))::bigint AS "coalesce"
           FROM public.payloads) AS payload_bytes,
    ( SELECT count(*) AS count
           FROM public.restore_jobs
          WHERE (restore_jobs.status = ANY (ARRAY['queued'::text, 'running'::text]))) AS queued_restores;

CREATE VIEW public.admin_restore_job_metadata AS
 SELECT id,
    project_id,
    requested_by,
    snapshot_id,
    target_change_sequence,
    reason,
    status,
    pre_restore_snapshot_id,
    generation_before,
    generation_after,
    restored_objects,
    restored_tombstones,
    error_code,
    created_at,
    started_at,
    finished_at,
    attempts,
    run_after,
    cancel_requested
   FROM public.restore_jobs;

CREATE TABLE public.snapshots (
    id uuid NOT NULL,
    project_id uuid NOT NULL,
    owner_user_id uuid NOT NULL,
    generation bigint NOT NULL,
    change_sequence bigint NOT NULL,
    manifest_hash character(64) NOT NULL,
    manifest_bucket text NOT NULL,
    manifest_key text NOT NULL,
    manifest jsonb NOT NULL,
    status text DEFAULT 'ready'::text NOT NULL,
    created_by uuid NOT NULL,
    created_at timestamp with time zone DEFAULT CURRENT_TIMESTAMP NOT NULL,
    CONSTRAINT snapshots_change_sequence_check CHECK ((change_sequence >= 0)),
    CONSTRAINT snapshots_generation_check CHECK ((generation > 0)),
    CONSTRAINT snapshots_status_check CHECK ((status = ANY (ARRAY['pending'::text, 'ready'::text, 'failed'::text])))
);

ALTER TABLE ONLY public.snapshots FORCE ROW LEVEL SECURITY;

CREATE VIEW public.admin_snapshot_metadata AS
 SELECT id,
    project_id,
    status
   FROM public.snapshots;

CREATE TABLE public.audit_events (
    id bigint NOT NULL,
    actor_user_id uuid,
    subject_user_id uuid,
    project_id uuid,
    action text NOT NULL,
    metadata jsonb DEFAULT '{}'::jsonb NOT NULL,
    request_id text,
    created_at timestamp with time zone DEFAULT CURRENT_TIMESTAMP NOT NULL
);

ALTER TABLE ONLY public.audit_events FORCE ROW LEVEL SECURITY;

CREATE TABLE public.admin_reauth_nonces (
    nonce_hash bytea NOT NULL,
    user_id uuid NOT NULL,
    device_id uuid NOT NULL,
    expires_at timestamp with time zone NOT NULL
);

ALTER TABLE ONLY public.admin_reauth_nonces FORCE ROW LEVEL SECURITY;

CREATE SEQUENCE public.audit_events_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE public.audit_events_id_seq OWNED BY public.audit_events.id;

CREATE TABLE public.bootstrap_completions (
    project_id uuid NOT NULL,
    owner_user_id uuid NOT NULL,
    device_id uuid NOT NULL,
    generation bigint NOT NULL,
    change_sequence bigint NOT NULL,
    completed_at timestamp with time zone DEFAULT CURRENT_TIMESTAMP NOT NULL,
    CONSTRAINT bootstrap_completions_change_sequence_check CHECK ((change_sequence >= 0)),
    CONSTRAINT bootstrap_completions_generation_check CHECK ((generation > 0))
);

ALTER TABLE ONLY public.bootstrap_completions FORCE ROW LEVEL SECURITY;

CREATE TABLE public.bootstrap_manifests (
    id uuid NOT NULL,
    project_id uuid NOT NULL,
    owner_user_id uuid NOT NULL,
    generation bigint NOT NULL,
    change_sequence bigint NOT NULL,
    manifest_hash character(64) NOT NULL,
    manifest_bucket text NOT NULL,
    manifest_key text NOT NULL,
    manifest jsonb NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    created_at timestamp with time zone DEFAULT CURRENT_TIMESTAMP NOT NULL,
    CONSTRAINT bootstrap_manifests_change_sequence_check CHECK ((change_sequence >= 0)),
    CONSTRAINT bootstrap_manifests_generation_check CHECK ((generation > 0))
);

ALTER TABLE ONLY public.bootstrap_manifests FORCE ROW LEVEL SECURITY;

CREATE TABLE public.distributed_rate_limit_buckets (
    bucket_hash bytea NOT NULL,
    window_started timestamp with time zone DEFAULT CURRENT_TIMESTAMP NOT NULL,
    request_count integer NOT NULL,
    CONSTRAINT distributed_rate_limit_buckets_request_count_check CHECK ((request_count > 0))
);

CREATE TABLE public.idempotency_records (
    owner_user_id uuid NOT NULL,
    device_id uuid NOT NULL,
    project_id uuid NOT NULL,
    request_key text NOT NULL,
    request_hash character(64) NOT NULL,
    response jsonb NOT NULL,
    created_at timestamp with time zone DEFAULT CURRENT_TIMESTAMP NOT NULL,
    expires_at timestamp with time zone DEFAULT (CURRENT_TIMESTAMP + '30 days'::interval) NOT NULL,
    response_status smallint DEFAULT 200 NOT NULL,
    CONSTRAINT idempotency_records_response_status_check CHECK (((response_status >= 200) AND (response_status <= 599)))
);

ALTER TABLE ONLY public.idempotency_records FORCE ROW LEVEL SECURITY;

CREATE TABLE public.instance_settings (
    key text NOT NULL,
    value jsonb NOT NULL,
    updated_at timestamp with time zone DEFAULT CURRENT_TIMESTAMP NOT NULL
);

CREATE TABLE public.invitations (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    email_normalized text NOT NULL,
    email_display text NOT NULL,
    token_hash bytea NOT NULL,
    role public.user_role DEFAULT 'user'::public.user_role NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    created_at timestamp with time zone DEFAULT CURRENT_TIMESTAMP NOT NULL,
    used_at timestamp with time zone,
    revoked_at timestamp with time zone
);

ALTER TABLE ONLY public.invitations FORCE ROW LEVEL SECURITY;

CREATE TABLE public.object_heads (
    project_id uuid NOT NULL,
    owner_user_id uuid NOT NULL,
    kind public.sync_object_kind NOT NULL,
    object_id text NOT NULL,
    revision bigint NOT NULL,
    revision_id uuid NOT NULL,
    CONSTRAINT object_heads_revision_check CHECK ((revision > 0))
);

ALTER TABLE ONLY public.object_heads FORCE ROW LEVEL SECURITY;

CREATE TABLE public.refresh_tokens (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    user_id uuid NOT NULL,
    device_id uuid NOT NULL,
    token_hash bytea NOT NULL,
    family_id uuid NOT NULL,
    issued_at timestamp with time zone DEFAULT CURRENT_TIMESTAMP NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    used_at timestamp with time zone,
    revoked_at timestamp with time zone
);

ALTER TABLE ONLY public.refresh_tokens FORCE ROW LEVEL SECURITY;

CREATE TABLE public.sync_attempts (
    id uuid DEFAULT gen_random_uuid() NOT NULL,
    owner_user_id uuid NOT NULL,
    project_id uuid NOT NULL,
    device_id uuid,
    operation text NOT NULL,
    status text NOT NULL,
    error_code text,
    item_count integer DEFAULT 0 NOT NULL,
    latency_ms integer,
    created_at timestamp with time zone DEFAULT CURRENT_TIMESTAMP NOT NULL,
    CONSTRAINT sync_attempts_item_count_check CHECK ((item_count >= 0)),
    CONSTRAINT sync_attempts_latency_ms_check CHECK ((latency_ms >= 0)),
    CONSTRAINT sync_attempts_operation_check CHECK ((operation = ANY (ARRAY['bootstrap'::text, 'pull'::text, 'push'::text]))),
    CONSTRAINT sync_attempts_status_check CHECK ((status = ANY (ARRAY['succeeded'::text, 'conflict'::text, 'rejected'::text, 'failed'::text])))
);

ALTER TABLE ONLY public.sync_attempts FORCE ROW LEVEL SECURITY;

ALTER TABLE ONLY public.audit_events ALTER COLUMN id SET DEFAULT nextval('public.audit_events_id_seq'::regclass);

ALTER TABLE ONLY public.account_purge_exports
    ADD CONSTRAINT account_purge_exports_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.audit_events
    ADD CONSTRAINT audit_events_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.bootstrap_completions
    ADD CONSTRAINT bootstrap_completions_pkey PRIMARY KEY (project_id, device_id, generation);

ALTER TABLE ONLY public.bootstrap_manifests
    ADD CONSTRAINT bootstrap_manifests_id_project_id_key UNIQUE (id, project_id);

ALTER TABLE ONLY public.bootstrap_manifests
    ADD CONSTRAINT bootstrap_manifests_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.bootstrap_manifests
    ADD CONSTRAINT bootstrap_manifests_project_id_manifest_hash_key UNIQUE (project_id, manifest_hash);

ALTER TABLE ONLY public.change_log
    ADD CONSTRAINT change_log_pkey PRIMARY KEY (project_id, sequence);

ALTER TABLE ONLY public.change_log
    ADD CONSTRAINT change_log_revision_id_key UNIQUE (revision_id);

ALTER TABLE ONLY public.devices
    ADD CONSTRAINT devices_id_owner_user_id_key UNIQUE (id, owner_user_id);

ALTER TABLE ONLY public.devices
    ADD CONSTRAINT devices_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.distributed_rate_limit_buckets
    ADD CONSTRAINT distributed_rate_limit_buckets_pkey PRIMARY KEY (bucket_hash);

ALTER TABLE ONLY public.idempotency_records
    ADD CONSTRAINT idempotency_records_pkey PRIMARY KEY (owner_user_id, device_id, project_id, request_key);

ALTER TABLE ONLY public.instance_settings
    ADD CONSTRAINT instance_settings_pkey PRIMARY KEY (key);

ALTER TABLE ONLY public.invitations
    ADD CONSTRAINT invitations_email_normalized_key UNIQUE (email_normalized);

ALTER TABLE ONLY public.invitations
    ADD CONSTRAINT invitations_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.invitations
    ADD CONSTRAINT invitations_token_hash_key UNIQUE (token_hash);

ALTER TABLE ONLY public.jobs
    ADD CONSTRAINT jobs_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.object_heads
    ADD CONSTRAINT object_heads_pkey PRIMARY KEY (project_id, kind, object_id);

ALTER TABLE ONLY public.object_revisions
    ADD CONSTRAINT object_revisions_id_project_id_key UNIQUE (id, project_id);

ALTER TABLE ONLY public.object_revisions
    ADD CONSTRAINT object_revisions_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.object_revisions
    ADD CONSTRAINT object_revisions_project_id_kind_object_id_revision_key UNIQUE (project_id, kind, object_id, revision);

ALTER TABLE ONLY public.payloads
    ADD CONSTRAINT payloads_id_project_id_key UNIQUE (id, project_id);

ALTER TABLE ONLY public.payloads
    ADD CONSTRAINT payloads_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.payloads
    ADD CONSTRAINT payloads_project_id_content_hash_key UNIQUE (project_id, content_hash);

ALTER TABLE ONLY public.projects
    ADD CONSTRAINT projects_id_owner_user_id_key UNIQUE (id, owner_user_id);

ALTER TABLE ONLY public.projects
    ADD CONSTRAINT projects_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.refresh_tokens
    ADD CONSTRAINT refresh_tokens_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.refresh_tokens
    ADD CONSTRAINT refresh_tokens_token_hash_key UNIQUE (token_hash);

ALTER TABLE ONLY public.restore_jobs
    ADD CONSTRAINT restore_jobs_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.snapshots
    ADD CONSTRAINT snapshots_id_project_id_key UNIQUE (id, project_id);

ALTER TABLE ONLY public.snapshots
    ADD CONSTRAINT snapshots_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.snapshots
    ADD CONSTRAINT snapshots_project_id_manifest_hash_key UNIQUE (project_id, manifest_hash);

ALTER TABLE ONLY public.sync_attempts
    ADD CONSTRAINT sync_attempts_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.users
    ADD CONSTRAINT users_email_normalized_key UNIQUE (email_normalized);

ALTER TABLE ONLY public.users
    ADD CONSTRAINT users_pkey PRIMARY KEY (id);

CREATE INDEX account_purge_exports_expiry_idx ON public.account_purge_exports USING btree (status, expires_at);

CREATE INDEX account_purge_exports_user_idx ON public.account_purge_exports USING btree (user_id, created_at DESC);

CREATE INDEX admin_reauth_nonces_expiry_idx ON public.admin_reauth_nonces USING btree (expires_at);

CREATE INDEX bootstrap_manifests_expiry_idx ON public.bootstrap_manifests USING btree (expires_at);

CREATE INDEX change_log_sequence_idx ON public.change_log USING btree (project_id, sequence);

CREATE INDEX devices_owner_idx ON public.devices USING btree (owner_user_id);

CREATE INDEX distributed_rate_limit_buckets_window_idx ON public.distributed_rate_limit_buckets USING btree (window_started);

CREATE INDEX idempotency_records_expiry_idx ON public.idempotency_records USING btree (expires_at);

CREATE INDEX invitations_status_idx ON public.invitations USING btree (revoked_at, used_at, expires_at);

CREATE UNIQUE INDEX jobs_one_account_purge_per_user ON public.jobs USING btree (owner_user_id) WHERE ((kind = 'account_purge'::text) AND (status = ANY (ARRAY['queued'::text, 'running'::text])));

CREATE INDEX jobs_project_idx ON public.jobs USING btree (project_id, created_at DESC);

CREATE INDEX jobs_queue_idx ON public.jobs USING btree (kind, status, run_after, created_at);

CREATE INDEX object_revisions_history_idx ON public.object_revisions USING btree (project_id, kind, object_id, revision DESC);

CREATE INDEX payloads_created_idx ON public.payloads USING btree (created_at);

CREATE INDEX projects_owner_idx ON public.projects USING btree (owner_user_id);

CREATE INDEX refresh_tokens_family_idx ON public.refresh_tokens USING btree (family_id);

CREATE INDEX restore_jobs_lease_idx ON public.restore_jobs USING btree (status, lease_expires_at) WHERE (status = 'running'::text);

CREATE UNIQUE INDEX restore_jobs_one_active_per_project ON public.restore_jobs USING btree (project_id) WHERE (status = ANY (ARRAY['queued'::text, 'running'::text]));

CREATE INDEX restore_jobs_project_idx ON public.restore_jobs USING btree (project_id, created_at DESC);

CREATE INDEX restore_jobs_queue_idx ON public.restore_jobs USING btree (status, created_at);

CREATE INDEX restore_jobs_retry_idx ON public.restore_jobs USING btree (status, run_after, created_at);

CREATE INDEX snapshots_project_created_idx ON public.snapshots USING btree (project_id, created_at DESC);

CREATE INDEX sync_attempts_created_idx ON public.sync_attempts USING btree (created_at DESC);

CREATE INDEX sync_attempts_project_idx ON public.sync_attempts USING btree (project_id, created_at DESC);

ALTER TABLE ONLY public.account_purge_exports
    ADD CONSTRAINT account_purge_exports_job_id_fkey FOREIGN KEY (job_id) REFERENCES public.jobs(id);

ALTER TABLE ONLY public.account_purge_exports
    ADD CONSTRAINT account_purge_exports_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id);

ALTER TABLE ONLY public.audit_events
    ADD CONSTRAINT audit_events_actor_user_id_fkey FOREIGN KEY (actor_user_id) REFERENCES public.users(id);

ALTER TABLE ONLY public.audit_events
    ADD CONSTRAINT audit_events_project_id_fkey FOREIGN KEY (project_id) REFERENCES public.projects(id) ON DELETE SET NULL;

ALTER TABLE ONLY public.audit_events
    ADD CONSTRAINT audit_events_subject_user_id_fkey FOREIGN KEY (subject_user_id) REFERENCES public.users(id);

ALTER TABLE ONLY public.admin_reauth_nonces
    ADD CONSTRAINT admin_reauth_nonces_pkey PRIMARY KEY (nonce_hash);

ALTER TABLE ONLY public.admin_reauth_nonces
    ADD CONSTRAINT admin_reauth_nonces_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id);

ALTER TABLE ONLY public.admin_reauth_nonces
    ADD CONSTRAINT admin_reauth_nonces_device_id_user_id_fkey FOREIGN KEY (device_id, user_id) REFERENCES public.devices(id, owner_user_id);

ALTER TABLE ONLY public.bootstrap_completions
    ADD CONSTRAINT bootstrap_completions_device_id_owner_user_id_fkey FOREIGN KEY (device_id, owner_user_id) REFERENCES public.devices(id, owner_user_id);

ALTER TABLE ONLY public.bootstrap_completions
    ADD CONSTRAINT bootstrap_completions_project_id_owner_user_id_fkey FOREIGN KEY (project_id, owner_user_id) REFERENCES public.projects(id, owner_user_id);

ALTER TABLE ONLY public.bootstrap_manifests
    ADD CONSTRAINT bootstrap_manifests_project_id_owner_user_id_fkey FOREIGN KEY (project_id, owner_user_id) REFERENCES public.projects(id, owner_user_id);

ALTER TABLE ONLY public.change_log
    ADD CONSTRAINT change_log_project_id_owner_user_id_fkey FOREIGN KEY (project_id, owner_user_id) REFERENCES public.projects(id, owner_user_id);

ALTER TABLE ONLY public.change_log
    ADD CONSTRAINT change_log_revision_id_project_id_fkey FOREIGN KEY (revision_id, project_id) REFERENCES public.object_revisions(id, project_id);

ALTER TABLE ONLY public.devices
    ADD CONSTRAINT devices_owner_user_id_fkey FOREIGN KEY (owner_user_id) REFERENCES public.users(id);

ALTER TABLE ONLY public.idempotency_records
    ADD CONSTRAINT idempotency_records_device_id_owner_user_id_fkey FOREIGN KEY (device_id, owner_user_id) REFERENCES public.devices(id, owner_user_id);

ALTER TABLE ONLY public.idempotency_records
    ADD CONSTRAINT idempotency_records_owner_user_id_fkey FOREIGN KEY (owner_user_id) REFERENCES public.users(id);

ALTER TABLE ONLY public.idempotency_records
    ADD CONSTRAINT idempotency_records_project_id_owner_user_id_fkey FOREIGN KEY (project_id, owner_user_id) REFERENCES public.projects(id, owner_user_id);

ALTER TABLE ONLY public.object_heads
    ADD CONSTRAINT object_heads_project_id_owner_user_id_fkey FOREIGN KEY (project_id, owner_user_id) REFERENCES public.projects(id, owner_user_id);

ALTER TABLE ONLY public.object_heads
    ADD CONSTRAINT object_heads_revision_id_project_id_fkey FOREIGN KEY (revision_id, project_id) REFERENCES public.object_revisions(id, project_id);

ALTER TABLE ONLY public.object_revisions
    ADD CONSTRAINT object_revisions_device_id_owner_user_id_fkey FOREIGN KEY (device_id, owner_user_id) REFERENCES public.devices(id, owner_user_id);

ALTER TABLE ONLY public.object_revisions
    ADD CONSTRAINT object_revisions_payload_id_project_id_fkey FOREIGN KEY (payload_id, project_id) REFERENCES public.payloads(id, project_id);

ALTER TABLE ONLY public.object_revisions
    ADD CONSTRAINT object_revisions_project_id_owner_user_id_fkey FOREIGN KEY (project_id, owner_user_id) REFERENCES public.projects(id, owner_user_id);

ALTER TABLE ONLY public.payloads
    ADD CONSTRAINT payloads_project_id_owner_user_id_fkey FOREIGN KEY (project_id, owner_user_id) REFERENCES public.projects(id, owner_user_id);

ALTER TABLE ONLY public.projects
    ADD CONSTRAINT projects_owner_user_id_fkey FOREIGN KEY (owner_user_id) REFERENCES public.users(id);

ALTER TABLE ONLY public.refresh_tokens
    ADD CONSTRAINT refresh_tokens_device_id_user_id_fkey FOREIGN KEY (device_id, user_id) REFERENCES public.devices(id, owner_user_id);

ALTER TABLE ONLY public.refresh_tokens
    ADD CONSTRAINT refresh_tokens_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id);

ALTER TABLE ONLY public.restore_jobs
    ADD CONSTRAINT restore_jobs_pre_restore_snapshot_id_project_id_fkey FOREIGN KEY (pre_restore_snapshot_id, project_id) REFERENCES public.snapshots(id, project_id);

ALTER TABLE ONLY public.restore_jobs
    ADD CONSTRAINT restore_jobs_project_id_owner_user_id_fkey FOREIGN KEY (project_id, owner_user_id) REFERENCES public.projects(id, owner_user_id);

ALTER TABLE ONLY public.restore_jobs
    ADD CONSTRAINT restore_jobs_requested_by_fkey FOREIGN KEY (requested_by) REFERENCES public.users(id);

ALTER TABLE ONLY public.restore_jobs
    ADD CONSTRAINT restore_jobs_snapshot_id_project_id_fkey FOREIGN KEY (snapshot_id, project_id) REFERENCES public.snapshots(id, project_id);

ALTER TABLE ONLY public.snapshots
    ADD CONSTRAINT snapshots_created_by_fkey FOREIGN KEY (created_by) REFERENCES public.users(id);

ALTER TABLE ONLY public.snapshots
    ADD CONSTRAINT snapshots_project_id_owner_user_id_fkey FOREIGN KEY (project_id, owner_user_id) REFERENCES public.projects(id, owner_user_id);

ALTER TABLE ONLY public.sync_attempts
    ADD CONSTRAINT sync_attempts_device_id_owner_user_id_fkey FOREIGN KEY (device_id, owner_user_id) REFERENCES public.devices(id, owner_user_id);

ALTER TABLE ONLY public.sync_attempts
    ADD CONSTRAINT sync_attempts_owner_user_id_fkey FOREIGN KEY (owner_user_id) REFERENCES public.users(id);

ALTER TABLE ONLY public.sync_attempts
    ADD CONSTRAINT sync_attempts_project_id_owner_user_id_fkey FOREIGN KEY (project_id, owner_user_id) REFERENCES public.projects(id, owner_user_id);

ALTER TABLE public.account_purge_exports ENABLE ROW LEVEL SECURITY;

CREATE POLICY account_purge_exports_admin ON public.account_purge_exports USING ((CURRENT_USER = 'tasktips_admin'::name)) WITH CHECK ((CURRENT_USER = 'tasktips_admin'::name));

CREATE POLICY account_purge_exports_worker ON public.account_purge_exports USING ((CURRENT_USER = 'tasktips_worker'::name)) WITH CHECK ((CURRENT_USER = 'tasktips_worker'::name));

ALTER TABLE public.audit_events ENABLE ROW LEVEL SECURITY;

CREATE POLICY audit_events_admin ON public.audit_events USING ((current_setting('app.role'::text, true) = 'system_admin'::text)) WITH CHECK ((current_setting('app.role'::text, true) = 'system_admin'::text));

CREATE POLICY audit_events_admin_role ON public.audit_events USING ((CURRENT_USER = 'tasktips_admin'::name)) WITH CHECK ((CURRENT_USER = 'tasktips_admin'::name));

CREATE POLICY audit_events_internal ON public.audit_events USING ((CURRENT_USER = 'tasktips_auth'::name)) WITH CHECK ((CURRENT_USER = 'tasktips_auth'::name));

CREATE POLICY audit_events_owner ON public.audit_events FOR INSERT WITH CHECK ((actor_user_id = public.app_current_user_id()));

CREATE POLICY audit_events_worker ON public.audit_events FOR INSERT WITH CHECK ((CURRENT_USER = 'tasktips_worker'::name));

CREATE POLICY audit_events_worker_update ON public.audit_events FOR UPDATE USING ((CURRENT_USER = 'tasktips_worker'::name)) WITH CHECK ((CURRENT_USER = 'tasktips_worker'::name));

ALTER TABLE public.admin_reauth_nonces ENABLE ROW LEVEL SECURITY;

CREATE POLICY admin_reauth_nonces_admin ON public.admin_reauth_nonces USING ((CURRENT_USER = 'tasktips_admin'::name)) WITH CHECK ((CURRENT_USER = 'tasktips_admin'::name));

ALTER TABLE public.bootstrap_completions ENABLE ROW LEVEL SECURITY;

CREATE POLICY bootstrap_completions_owner ON public.bootstrap_completions USING ((owner_user_id = public.app_current_user_id())) WITH CHECK ((owner_user_id = public.app_current_user_id()));

CREATE POLICY bootstrap_completions_worker_purge ON public.bootstrap_completions USING ((CURRENT_USER = 'tasktips_worker'::name)) WITH CHECK ((CURRENT_USER = 'tasktips_worker'::name));

ALTER TABLE public.bootstrap_manifests ENABLE ROW LEVEL SECURITY;

CREATE POLICY bootstrap_manifests_owner ON public.bootstrap_manifests USING ((owner_user_id = public.app_current_user_id())) WITH CHECK ((owner_user_id = public.app_current_user_id()));

CREATE POLICY bootstrap_manifests_worker ON public.bootstrap_manifests USING ((CURRENT_USER = 'tasktips_worker'::name)) WITH CHECK ((CURRENT_USER = 'tasktips_worker'::name));

CREATE POLICY bootstrap_manifests_worker_purge ON public.bootstrap_manifests USING ((CURRENT_USER = 'tasktips_worker'::name)) WITH CHECK ((CURRENT_USER = 'tasktips_worker'::name));

ALTER TABLE public.change_log ENABLE ROW LEVEL SECURITY;

CREATE POLICY change_log_admin ON public.change_log USING ((CURRENT_USER = 'tasktips_admin'::name)) WITH CHECK ((CURRENT_USER = 'tasktips_admin'::name));

CREATE POLICY change_log_owner ON public.change_log USING ((owner_user_id = public.app_current_user_id())) WITH CHECK ((owner_user_id = public.app_current_user_id()));

CREATE POLICY change_log_worker ON public.change_log USING ((CURRENT_USER = 'tasktips_worker'::name)) WITH CHECK ((CURRENT_USER = 'tasktips_worker'::name));

CREATE POLICY change_log_worker_delete ON public.change_log FOR DELETE USING ((CURRENT_USER = 'tasktips_worker'::name));

ALTER TABLE public.devices ENABLE ROW LEVEL SECURITY;

CREATE POLICY devices_internal ON public.devices USING ((CURRENT_USER = 'tasktips_auth'::name)) WITH CHECK ((CURRENT_USER = 'tasktips_auth'::name));

CREATE POLICY devices_owner ON public.devices USING ((owner_user_id = public.app_current_user_id())) WITH CHECK ((owner_user_id = public.app_current_user_id()));

CREATE POLICY devices_worker_account_purge ON public.devices USING ((CURRENT_USER = 'tasktips_worker'::name)) WITH CHECK ((CURRENT_USER = 'tasktips_worker'::name));

ALTER TABLE public.idempotency_records ENABLE ROW LEVEL SECURITY;

CREATE POLICY idempotency_records_owner ON public.idempotency_records USING ((owner_user_id = public.app_current_user_id())) WITH CHECK ((owner_user_id = public.app_current_user_id()));

CREATE POLICY idempotency_records_worker ON public.idempotency_records USING ((CURRENT_USER = 'tasktips_worker'::name)) WITH CHECK ((CURRENT_USER = 'tasktips_worker'::name));

CREATE POLICY idempotency_records_worker_purge ON public.idempotency_records USING ((CURRENT_USER = 'tasktips_worker'::name)) WITH CHECK ((CURRENT_USER = 'tasktips_worker'::name));

ALTER TABLE public.invitations ENABLE ROW LEVEL SECURITY;

CREATE POLICY invitations_admin ON public.invitations USING ((current_setting('app.role'::text, true) = 'system_admin'::text)) WITH CHECK ((current_setting('app.role'::text, true) = 'system_admin'::text));

CREATE POLICY invitations_internal ON public.invitations USING ((CURRENT_USER = 'tasktips_auth'::name)) WITH CHECK ((CURRENT_USER = 'tasktips_auth'::name));

CREATE POLICY invitations_worker_account_purge ON public.invitations USING ((CURRENT_USER = 'tasktips_worker'::name)) WITH CHECK ((CURRENT_USER = 'tasktips_worker'::name));

ALTER TABLE public.jobs ENABLE ROW LEVEL SECURITY;

CREATE POLICY jobs_admin ON public.jobs USING ((CURRENT_USER = 'tasktips_admin'::name)) WITH CHECK ((CURRENT_USER = 'tasktips_admin'::name));

CREATE POLICY jobs_owner ON public.jobs USING ((owner_user_id = public.app_current_user_id())) WITH CHECK ((owner_user_id = public.app_current_user_id()));

CREATE POLICY jobs_worker ON public.jobs USING ((CURRENT_USER = 'tasktips_worker'::name)) WITH CHECK ((CURRENT_USER = 'tasktips_worker'::name));

ALTER TABLE public.object_heads ENABLE ROW LEVEL SECURITY;

CREATE POLICY object_heads_owner ON public.object_heads USING ((owner_user_id = public.app_current_user_id())) WITH CHECK ((owner_user_id = public.app_current_user_id()));

CREATE POLICY object_heads_worker ON public.object_heads USING ((CURRENT_USER = 'tasktips_worker'::name)) WITH CHECK ((CURRENT_USER = 'tasktips_worker'::name));

CREATE POLICY object_heads_worker_delete ON public.object_heads FOR DELETE USING ((CURRENT_USER = 'tasktips_worker'::name));

ALTER TABLE public.object_revisions ENABLE ROW LEVEL SECURITY;

CREATE POLICY object_revisions_admin ON public.object_revisions USING ((CURRENT_USER = 'tasktips_admin'::name)) WITH CHECK ((CURRENT_USER = 'tasktips_admin'::name));

CREATE POLICY object_revisions_owner ON public.object_revisions USING ((owner_user_id = public.app_current_user_id())) WITH CHECK ((owner_user_id = public.app_current_user_id()));

CREATE POLICY object_revisions_worker ON public.object_revisions USING ((CURRENT_USER = 'tasktips_worker'::name)) WITH CHECK ((CURRENT_USER = 'tasktips_worker'::name));

CREATE POLICY object_revisions_worker_delete ON public.object_revisions FOR DELETE USING ((CURRENT_USER = 'tasktips_worker'::name));

ALTER TABLE public.payloads ENABLE ROW LEVEL SECURITY;

CREATE POLICY payloads_admin ON public.payloads USING ((CURRENT_USER = 'tasktips_admin'::name)) WITH CHECK ((CURRENT_USER = 'tasktips_admin'::name));

CREATE POLICY payloads_owner ON public.payloads USING ((owner_user_id = public.app_current_user_id())) WITH CHECK ((owner_user_id = public.app_current_user_id()));

CREATE POLICY payloads_worker ON public.payloads USING ((CURRENT_USER = 'tasktips_worker'::name)) WITH CHECK ((CURRENT_USER = 'tasktips_worker'::name));

ALTER TABLE public.projects ENABLE ROW LEVEL SECURITY;

CREATE POLICY projects_admin ON public.projects USING ((CURRENT_USER = 'tasktips_admin'::name)) WITH CHECK ((CURRENT_USER = 'tasktips_admin'::name));

CREATE POLICY projects_owner ON public.projects USING ((owner_user_id = public.app_current_user_id())) WITH CHECK ((owner_user_id = public.app_current_user_id()));

CREATE POLICY projects_worker ON public.projects USING ((CURRENT_USER = 'tasktips_worker'::name)) WITH CHECK ((CURRENT_USER = 'tasktips_worker'::name));

CREATE POLICY projects_worker_delete ON public.projects FOR DELETE USING ((CURRENT_USER = 'tasktips_worker'::name));

ALTER TABLE public.refresh_tokens ENABLE ROW LEVEL SECURITY;

CREATE POLICY refresh_tokens_internal ON public.refresh_tokens USING ((CURRENT_USER = 'tasktips_auth'::name)) WITH CHECK ((CURRENT_USER = 'tasktips_auth'::name));

CREATE POLICY refresh_tokens_owner ON public.refresh_tokens USING ((user_id = public.app_current_user_id())) WITH CHECK ((user_id = public.app_current_user_id()));

CREATE POLICY refresh_tokens_worker_account_purge ON public.refresh_tokens USING ((CURRENT_USER = 'tasktips_worker'::name)) WITH CHECK ((CURRENT_USER = 'tasktips_worker'::name));

ALTER TABLE public.restore_jobs ENABLE ROW LEVEL SECURITY;

CREATE POLICY restore_jobs_admin ON public.restore_jobs USING ((CURRENT_USER = 'tasktips_admin'::name)) WITH CHECK ((CURRENT_USER = 'tasktips_admin'::name));

CREATE POLICY restore_jobs_owner ON public.restore_jobs USING ((owner_user_id = public.app_current_user_id())) WITH CHECK ((owner_user_id = public.app_current_user_id()));

CREATE POLICY restore_jobs_worker ON public.restore_jobs USING ((CURRENT_USER = 'tasktips_worker'::name)) WITH CHECK ((CURRENT_USER = 'tasktips_worker'::name));

CREATE POLICY restore_jobs_worker_purge ON public.restore_jobs USING ((CURRENT_USER = 'tasktips_worker'::name)) WITH CHECK ((CURRENT_USER = 'tasktips_worker'::name));

ALTER TABLE public.snapshots ENABLE ROW LEVEL SECURITY;

CREATE POLICY snapshots_admin ON public.snapshots USING ((CURRENT_USER = 'tasktips_admin'::name)) WITH CHECK ((CURRENT_USER = 'tasktips_admin'::name));

CREATE POLICY snapshots_owner ON public.snapshots USING ((owner_user_id = public.app_current_user_id())) WITH CHECK ((owner_user_id = public.app_current_user_id()));

CREATE POLICY snapshots_worker ON public.snapshots USING ((CURRENT_USER = 'tasktips_worker'::name)) WITH CHECK ((CURRENT_USER = 'tasktips_worker'::name));

CREATE POLICY snapshots_worker_purge ON public.snapshots USING ((CURRENT_USER = 'tasktips_worker'::name)) WITH CHECK ((CURRENT_USER = 'tasktips_worker'::name));

ALTER TABLE public.sync_attempts ENABLE ROW LEVEL SECURITY;

CREATE POLICY sync_attempts_admin ON public.sync_attempts USING ((CURRENT_USER = 'tasktips_admin'::name)) WITH CHECK ((CURRENT_USER = 'tasktips_admin'::name));

CREATE POLICY sync_attempts_owner ON public.sync_attempts USING ((owner_user_id = public.app_current_user_id())) WITH CHECK ((owner_user_id = public.app_current_user_id()));

CREATE POLICY sync_attempts_worker_purge ON public.sync_attempts USING ((CURRENT_USER = 'tasktips_worker'::name)) WITH CHECK ((CURRENT_USER = 'tasktips_worker'::name));

ALTER TABLE public.users ENABLE ROW LEVEL SECURITY;

CREATE POLICY users_admin_account_purge ON public.users USING ((CURRENT_USER = 'tasktips_admin'::name)) WITH CHECK ((CURRENT_USER = 'tasktips_admin'::name));

CREATE POLICY users_internal ON public.users USING ((CURRENT_USER = 'tasktips_auth'::name)) WITH CHECK ((CURRENT_USER = 'tasktips_auth'::name));

CREATE POLICY users_self ON public.users USING ((id = public.app_current_user_id())) WITH CHECK ((id = public.app_current_user_id()));

CREATE POLICY users_worker_account_purge ON public.users USING ((CURRENT_USER = 'tasktips_worker'::name)) WITH CHECK ((CURRENT_USER = 'tasktips_worker'::name));

GRANT USAGE ON SCHEMA public TO tasktips_auth;
GRANT USAGE ON SCHEMA public TO tasktips_app;
GRANT USAGE ON SCHEMA public TO tasktips_admin;
GRANT USAGE ON SCHEMA public TO tasktips_worker;

REVOKE ALL ON FUNCTION public.admin_lock_project(target_project_id uuid) FROM PUBLIC;
GRANT ALL ON FUNCTION public.admin_lock_project(target_project_id uuid) TO tasktips_admin;

REVOKE ALL ON FUNCTION public.app_current_user_id() FROM PUBLIC;
GRANT ALL ON FUNCTION public.app_current_user_id() TO tasktips_app;
GRANT ALL ON FUNCTION public.app_current_user_id() TO tasktips_auth;
GRANT ALL ON FUNCTION public.app_current_user_id() TO tasktips_admin;
GRANT ALL ON FUNCTION public.app_current_user_id() TO tasktips_worker;

GRANT SELECT,INSERT,UPDATE ON TABLE public.account_purge_exports TO tasktips_worker;
GRANT INSERT,UPDATE ON TABLE public.account_purge_exports TO tasktips_admin;

GRANT SELECT(id) ON TABLE public.account_purge_exports TO tasktips_admin;

GRANT SELECT(user_id) ON TABLE public.account_purge_exports TO tasktips_admin;

GRANT SELECT(job_id) ON TABLE public.account_purge_exports TO tasktips_admin;

GRANT SELECT(status) ON TABLE public.account_purge_exports TO tasktips_admin;

GRANT SELECT(created_at) ON TABLE public.account_purge_exports TO tasktips_admin;

GRANT SELECT(expires_at) ON TABLE public.account_purge_exports TO tasktips_admin;

GRANT SELECT(completed_at) ON TABLE public.account_purge_exports TO tasktips_admin;

GRANT SELECT ON TABLE public.admin_account_purge_export_metadata TO tasktips_admin;

GRANT SELECT,INSERT,DELETE ON TABLE public.admin_reauth_nonces TO tasktips_admin;

GRANT SELECT,INSERT,UPDATE ON TABLE public.devices TO tasktips_auth;
GRANT SELECT,INSERT,UPDATE ON TABLE public.devices TO tasktips_app;
GRANT SELECT,DELETE ON TABLE public.devices TO tasktips_worker;

GRANT SELECT ON TABLE public.admin_device_metadata TO tasktips_admin;

GRANT SELECT,INSERT ON TABLE public.change_log TO tasktips_app;
GRANT SELECT,INSERT,DELETE ON TABLE public.change_log TO tasktips_worker;

GRANT SELECT,INSERT,DELETE ON TABLE public.object_revisions TO tasktips_worker;
GRANT SELECT,INSERT ON TABLE public.object_revisions TO tasktips_app;

GRANT SELECT ON TABLE public.admin_history_metadata TO tasktips_admin;

GRANT SELECT,INSERT ON TABLE public.jobs TO tasktips_app;
GRANT SELECT,INSERT,UPDATE ON TABLE public.jobs TO tasktips_worker;
GRANT INSERT ON TABLE public.jobs TO tasktips_admin;

GRANT SELECT ON TABLE public.admin_job_metadata TO tasktips_admin;

GRANT SELECT,INSERT,UPDATE ON TABLE public.projects TO tasktips_app;
GRANT SELECT,DELETE,UPDATE ON TABLE public.projects TO tasktips_worker;
GRANT SELECT,UPDATE ON TABLE public.projects TO tasktips_admin;

GRANT SELECT ON TABLE public.admin_project_metadata TO tasktips_admin;

GRANT SELECT,INSERT,UPDATE ON TABLE public.users TO tasktips_auth;
GRANT SELECT,INSERT,UPDATE ON TABLE public.users TO tasktips_app;
GRANT SELECT,UPDATE ON TABLE public.users TO tasktips_worker;

GRANT SELECT(id) ON TABLE public.users TO tasktips_admin;

GRANT SELECT(email_normalized) ON TABLE public.users TO tasktips_admin;

GRANT SELECT(role) ON TABLE public.users TO tasktips_admin;

GRANT SELECT(status),UPDATE(status) ON TABLE public.users TO tasktips_admin;

GRANT SELECT ON TABLE public.admin_user_metadata TO tasktips_admin;

GRANT SELECT,DELETE ON TABLE public.payloads TO tasktips_worker;
GRANT SELECT,INSERT ON TABLE public.payloads TO tasktips_app;

GRANT SELECT,INSERT,UPDATE ON TABLE public.restore_jobs TO tasktips_app;
GRANT SELECT,INSERT,DELETE,UPDATE ON TABLE public.restore_jobs TO tasktips_worker;
GRANT INSERT ON TABLE public.restore_jobs TO tasktips_admin;

GRANT SELECT ON TABLE public.admin_operational_counts TO tasktips_admin;

GRANT SELECT ON TABLE public.admin_restore_job_metadata TO tasktips_admin;

GRANT SELECT,INSERT,UPDATE ON TABLE public.snapshots TO tasktips_app;
GRANT SELECT,INSERT,DELETE,UPDATE ON TABLE public.snapshots TO tasktips_worker;

GRANT SELECT ON TABLE public.admin_snapshot_metadata TO tasktips_admin;

GRANT SELECT,INSERT,UPDATE ON TABLE public.audit_events TO tasktips_auth;
GRANT SELECT,INSERT ON TABLE public.audit_events TO tasktips_admin;
GRANT INSERT ON TABLE public.audit_events TO tasktips_app;
GRANT INSERT,UPDATE ON TABLE public.audit_events TO tasktips_worker;

GRANT SELECT,USAGE ON SEQUENCE public.audit_events_id_seq TO tasktips_auth;
GRANT SELECT,USAGE ON SEQUENCE public.audit_events_id_seq TO tasktips_admin;
GRANT SELECT,USAGE ON SEQUENCE public.audit_events_id_seq TO tasktips_app;
GRANT SELECT,USAGE ON SEQUENCE public.audit_events_id_seq TO tasktips_worker;

GRANT SELECT,INSERT,UPDATE ON TABLE public.bootstrap_completions TO tasktips_app;
GRANT SELECT,DELETE ON TABLE public.bootstrap_completions TO tasktips_worker;

GRANT SELECT,INSERT ON TABLE public.bootstrap_manifests TO tasktips_app;
GRANT SELECT,DELETE ON TABLE public.bootstrap_manifests TO tasktips_worker;

GRANT SELECT,INSERT,UPDATE ON TABLE public.distributed_rate_limit_buckets TO tasktips_auth;
GRANT SELECT,INSERT,DELETE,UPDATE ON TABLE public.distributed_rate_limit_buckets TO tasktips_worker;

GRANT SELECT,INSERT,DELETE ON TABLE public.idempotency_records TO tasktips_app;
GRANT SELECT,DELETE ON TABLE public.idempotency_records TO tasktips_worker;

GRANT SELECT,INSERT,UPDATE ON TABLE public.invitations TO tasktips_auth;
GRANT SELECT,INSERT ON TABLE public.invitations TO tasktips_admin;
GRANT SELECT,DELETE ON TABLE public.invitations TO tasktips_worker;

GRANT SELECT,INSERT,UPDATE ON TABLE public.object_heads TO tasktips_app;
GRANT SELECT,INSERT,DELETE,UPDATE ON TABLE public.object_heads TO tasktips_worker;

GRANT SELECT,INSERT,UPDATE ON TABLE public.refresh_tokens TO tasktips_auth;
GRANT SELECT,INSERT,UPDATE ON TABLE public.refresh_tokens TO tasktips_app;
GRANT SELECT,DELETE ON TABLE public.refresh_tokens TO tasktips_worker;

GRANT SELECT,INSERT ON TABLE public.sync_attempts TO tasktips_app;
GRANT SELECT ON TABLE public.sync_attempts TO tasktips_admin;
GRANT SELECT,DELETE ON TABLE public.sync_attempts TO tasktips_worker;

-- Keep the phase markers used by readiness checks and operational tooling.
INSERT INTO public.instance_settings (key, value) VALUES
    ('identity_schema_version', '{"version":1}'::jsonb),
    ('sync_schema_version', '{"version":1}'::jsonb),
    ('history_schema_version', '{"version":1}'::jsonb),
    ('admin_schema_version', '{"version":1}'::jsonb),
    ('admin_safe_views_schema_version', '{"version":1}'::jsonb),
    ('restore_lease_schema_version', '{"version":1}'::jsonb),
    ('admin_project_lock_schema_version', '{"version":1}'::jsonb),
    ('restore_requeue_schema_version', '{"version":1}'::jsonb),
    ('restore_job_guard_schema_version', '{"version":1}'::jsonb),
    ('retention_cleanup_schema_version', '{"version":1}'::jsonb),
    ('invitation_operations_schema_version', '{"version":1}'::jsonb),
    ('idempotency_expiry_permissions_schema_version', '{"version":1}'::jsonb),
    ('worker_rls_function_access_schema_version', '{"version":1}'::jsonb),
    ('idempotency_project_scope_schema_version', '{"version":1}'::jsonb),
    ('admin_project_reopen_schema_version', '{"version":1}'::jsonb),
    ('bootstrap_manifest_schema_version', '{"version":1}'::jsonb),
    ('jobs_project_purge_schema_version', '{"version":1}'::jsonb),
    ('worker_purge_rls_schema_version', '{"version":1}'::jsonb),
    ('restore_retry_schema_version', '{"version":1}'::jsonb),
    ('account_purge_schema_version', '{"version":1}'::jsonb),
    ('account_purge_admin_rls_schema_version', '{"version":1}'::jsonb),
    ('restore_cancellation_schema_version', '{"version":1}'::jsonb),
    ('distributed_rate_limit_schema_version', '{"version":1}'::jsonb),
    ('admin_restore_progress_schema_version', '{"version":1}'::jsonb)
ON CONFLICT (key) DO NOTHING;
