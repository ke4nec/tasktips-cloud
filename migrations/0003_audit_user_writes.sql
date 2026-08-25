GRANT INSERT ON audit_events TO tasktips_app;
GRANT USAGE, SELECT ON SEQUENCE audit_events_id_seq TO tasktips_app;

CREATE POLICY audit_events_owner ON audit_events
    FOR INSERT
    WITH CHECK (
        actor_user_id = NULLIF(current_setting('app.user_id', true), '')::uuid
    );
