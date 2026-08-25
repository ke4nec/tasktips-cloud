CREATE POLICY audit_events_worker ON audit_events
    FOR INSERT
    WITH CHECK (CURRENT_USER = 'tasktips_worker');
