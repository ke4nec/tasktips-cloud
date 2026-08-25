CREATE OR REPLACE FUNCTION admin_lock_project(target_project_id UUID)
RETURNS TABLE(owner_user_id UUID, change_seq BIGINT, status project_status)
LANGUAGE sql
SECURITY DEFINER
SET search_path = public
AS $$
    SELECT p.owner_user_id, p.change_seq, p.status
    FROM projects AS p
    WHERE p.id = target_project_id
    FOR UPDATE
$$;

REVOKE ALL ON FUNCTION admin_lock_project(UUID) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION admin_lock_project(UUID) TO tasktips_admin;

INSERT INTO instance_settings (key, value)
VALUES ('admin_project_lock_schema_version', '{"version": 1}'::jsonb)
ON CONFLICT (key) DO NOTHING;
