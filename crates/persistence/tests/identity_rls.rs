use tasktips_persistence::{
    DeviceProfile, NewInvitation, NewRefreshToken, Persistence, PersistenceError,
    invitation_expiry, refresh_expiry,
};
use uuid::Uuid;

#[tokio::test]
async fn rls_and_auth_state_isolate_two_users() {
    let Ok(database_url) = std::env::var("TASKTIPS_DATABASE_URL") else {
        assert!(
            std::env::var_os("CI").is_none(),
            "TASKTIPS_DATABASE_URL is required for identity_rls in CI"
        );
        eprintln!("identity_rls skipped: TASKTIPS_DATABASE_URL is not set");
        return;
    };
    let persistence = Persistence::connect(&database_url)
        .await
        .expect("test database should be reachable");
    persistence
        .migrate()
        .await
        .expect("migrations should apply");

    let test_id = Uuid::new_v4();
    let admin = persistence
        .create_initial_admin(
            &format!("admin-{test_id}@example.test"),
            &format!("admin-{test_id}@example.test"),
            "test-password-hash",
        )
        .await
        .expect("admin should be created");
    let user_one = activate_user(&persistence, admin.id, test_id, 1).await;
    let user_two = activate_user(&persistence, admin.id, test_id, 2).await;

    let project_two_id = assert_project_isolation(&persistence, &user_one, &user_two).await;
    assert_device_isolation(&persistence, &user_one, &user_two).await;
    assert_admin_boundary(&persistence, admin.id, &user_one, &user_two, project_two_id).await;
    assert_refresh_reuse_revokes_family(&persistence, &user_one, test_id).await;
    assert_device_revocation(&persistence, &user_two).await;
}

#[tokio::test]
async fn malformed_request_identity_fails_closed_under_rls() {
    let Ok(database_url) = std::env::var("TASKTIPS_DATABASE_URL") else {
        assert!(
            std::env::var_os("CI").is_none(),
            "TASKTIPS_DATABASE_URL is required for identity_rls in CI"
        );
        eprintln!("identity_rls skipped: TASKTIPS_DATABASE_URL is not set");
        return;
    };
    let persistence = Persistence::connect(&database_url)
        .await
        .expect("test database should be reachable");
    persistence
        .migrate()
        .await
        .expect("migrations should apply");
    let mut connection = persistence
        .pool()
        .acquire()
        .await
        .expect("database connection should be available");
    sqlx::query("BEGIN")
        .execute(&mut *connection)
        .await
        .expect("transaction should begin");
    sqlx::query("SET LOCAL ROLE tasktips_app")
        .execute(&mut *connection)
        .await
        .expect("application role should be available");
    sqlx::query("SELECT set_config('app.user_id', 'not-a-uuid', true)")
        .execute(&mut *connection)
        .await
        .expect("request identity should be set");
    let visible: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM projects")
        .fetch_one(&mut *connection)
        .await
        .expect("malformed identity must not abort the query");
    assert_eq!(visible, 0);
    sqlx::query("ROLLBACK")
        .execute(&mut *connection)
        .await
        .expect("transaction should roll back");
}

#[tokio::test]
async fn distributed_rate_limit_is_shared_and_windowed() {
    let Ok(database_url) = std::env::var("TASKTIPS_DATABASE_URL") else {
        assert!(
            std::env::var_os("CI").is_none(),
            "TASKTIPS_DATABASE_URL is required for distributed rate-limit test in CI"
        );
        eprintln!("distributed rate-limit test skipped: TASKTIPS_DATABASE_URL is not set");
        return;
    };
    let persistence = Persistence::connect(&database_url)
        .await
        .expect("test database should be reachable");
    persistence
        .migrate()
        .await
        .expect("migrations should apply");
    let bucket = format!("rate-limit-test-{}", Uuid::new_v4());
    assert!(
        persistence
            .consume_distributed_rate_limit(&bucket, 2, time::Duration::seconds(60))
            .await
            .expect("first token should be accepted")
    );
    assert!(
        persistence
            .consume_distributed_rate_limit(&bucket, 2, time::Duration::seconds(60))
            .await
            .expect("second token should be accepted")
    );
    assert!(
        !persistence
            .consume_distributed_rate_limit(&bucket, 2, time::Duration::seconds(60))
            .await
            .expect("third token should be rejected")
    );
}

#[tokio::test]
async fn reauth_nonce_is_shared_and_one_use() {
    let Ok(database_url) = std::env::var("TASKTIPS_DATABASE_URL") else {
        assert!(
            std::env::var_os("CI").is_none(),
            "TASKTIPS_DATABASE_URL is required for re-auth nonce test in CI"
        );
        eprintln!("reauth nonce test skipped: TASKTIPS_DATABASE_URL is not set");
        return;
    };
    let persistence = Persistence::connect(&database_url)
        .await
        .expect("test database should be reachable");
    persistence
        .migrate()
        .await
        .expect("migrations should apply");

    let test_id = Uuid::new_v4();
    let admin = persistence
        .create_initial_admin(
            &format!("reauth-admin-{test_id}@example.test"),
            &format!("reauth-admin-{test_id}@example.test"),
            "test-password-hash",
        )
        .await
        .expect("admin should be created");
    let device_id = Uuid::new_v4();
    persistence
        .create_login_session(
            admin.id,
            device_id,
            &NewRefreshToken {
                token_hash: token_hash(test_id, 41),
                family_id: Uuid::new_v4(),
                expires_at: refresh_expiry(3_600),
            },
            "reauth-session-test",
        )
        .await
        .expect("admin device should be registered");

    let nonce_hash = vec![41_u8; 32];
    persistence
        .create_reauth_nonce(
            admin.id,
            device_id,
            &nonce_hash,
            time::OffsetDateTime::now_utc() + time::Duration::minutes(5),
        )
        .await
        .expect("nonce should be persisted");
    let second_connection = Persistence::connect(&database_url)
        .await
        .expect("second database connection should be reachable");
    assert!(
        second_connection
            .consume_reauth_nonce(admin.id, device_id, &nonce_hash)
            .await
            .expect("nonce consumption should succeed")
    );
    assert!(
        !second_connection
            .consume_reauth_nonce(admin.id, device_id, &nonce_hash)
            .await
            .expect("reusing a nonce should be rejected")
    );
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn account_purge_requires_export_and_finishes_as_deleted() {
    let Ok(database_url) = std::env::var("TASKTIPS_DATABASE_URL") else {
        assert!(
            std::env::var_os("CI").is_none(),
            "TASKTIPS_DATABASE_URL is required for account purge integration test in CI"
        );
        eprintln!("account purge integration test skipped: TASKTIPS_DATABASE_URL is not set");
        return;
    };
    let persistence = Persistence::connect(&database_url)
        .await
        .expect("test database should be reachable");
    persistence
        .migrate()
        .await
        .expect("migrations should apply");
    let test_id = Uuid::new_v4();
    let admin = persistence
        .create_initial_admin(
            &format!("purge-admin-{test_id}@example.test"),
            &format!("purge-admin-{test_id}@example.test"),
            "test-password-hash",
        )
        .await
        .expect("admin should be created");
    let user = activate_user(&persistence, admin.id, test_id, 7).await;

    let first = persistence
        .request_account_purge_export(admin.id, user.user_id, "test export", "purge-test")
        .await
        .expect("first phase should queue an export");
    assert!(first.confirmation_required);
    assert_eq!(first.status, "queued");
    sqlx::query(
        "UPDATE jobs SET run_after = CURRENT_TIMESTAMP + INTERVAL '1 hour', \
             lease_expires_at = CURRENT_TIMESTAMP + INTERVAL '1 hour' \
         WHERE kind = 'account_purge' AND status IN ('queued', 'running') AND id <> $1",
    )
    .bind(first.job_id)
    .execute(persistence.pool())
    .await
    .expect("unrelated account purge jobs should not race the focused test");
    let export_job = persistence
        .claim_account_purge_job()
        .await
        .expect("export job should be claimable")
        .expect("export job should exist");
    let export_lease = export_job
        .lease_token
        .expect("export lease should be present");
    let export = persistence
        .prepare_account_export(export_job.id, export_lease)
        .await
        .expect("export should read account metadata");
    assert_eq!(export.owner_user_id, user.user_id);
    persistence
        .complete_account_export(
            export_job.id,
            export_lease,
            export.export_id,
            &"a".repeat(64),
            1,
        )
        .await
        .expect("export should become ready");

    let confirmed = persistence
        .confirm_account_purge(
            admin.id,
            user.user_id,
            first.export_id,
            "test confirm",
            "purge-confirm-test",
        )
        .await
        .expect("ready export should allow confirmation");
    assert!(!confirmed.confirmation_required);
    sqlx::query(
        "UPDATE jobs SET run_after = CURRENT_TIMESTAMP + INTERVAL '1 hour', \
             lease_expires_at = CURRENT_TIMESTAMP + INTERVAL '1 hour' \
         WHERE kind = 'account_purge' AND status IN ('queued', 'running') AND id <> $1",
    )
    .bind(confirmed.job_id)
    .execute(persistence.pool())
    .await
    .expect("unrelated account purge jobs should not race the focused test");
    let purge_job = persistence
        .claim_account_purge_job()
        .await
        .expect("purge job should be claimable")
        .expect("purge job should exist");
    let purge_lease = purge_job
        .lease_token
        .expect("purge lease should be present");
    let keys = persistence
        .prepare_account_purge(purge_job.id, purge_lease)
        .await
        .expect("account rows should be detached");
    assert!(
        keys.iter()
            .any(|key| key.contains(&first.export_id.to_string()))
    );
    persistence
        .complete_account_purge(purge_job.id, purge_lease)
        .await
        .expect("account should become deleted");
    let users = persistence
        .admin_list_users(admin.id)
        .await
        .expect("admin metadata should remain available");
    let deleted = users
        .into_iter()
        .find(|record| record.id == user.user_id)
        .expect("deleted user should retain an audit reference");
    assert_eq!(deleted.status, "deleted");
    assert_eq!(deleted.email, "deleted");
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn restore_cancellation_fences_queued_and_running_jobs() {
    let Ok(database_url) = std::env::var("TASKTIPS_DATABASE_URL") else {
        assert!(
            std::env::var_os("CI").is_none(),
            "TASKTIPS_DATABASE_URL is required for restore cancellation integration test in CI"
        );
        eprintln!(
            "restore cancellation integration test skipped: TASKTIPS_DATABASE_URL is not set"
        );
        return;
    };
    let persistence = Persistence::connect(&database_url)
        .await
        .expect("test database should be reachable");
    persistence
        .migrate()
        .await
        .expect("migrations should apply");
    let test_id = Uuid::new_v4();
    let admin = persistence
        .create_initial_admin(
            &format!("restore-admin-{test_id}@example.test"),
            &format!("restore-admin-{test_id}@example.test"),
            "test-password-hash",
        )
        .await
        .expect("admin should be created");
    let user = activate_user(&persistence, admin.id, test_id, 8).await;
    let project = persistence
        .create_project(user.user_id, "user", "Restore cancellation")
        .await
        .expect("project should be created");

    let queued = persistence
        .enqueue_restore(
            user.user_id,
            "user",
            project.id,
            None,
            Some(0),
            "queued cancellation",
            "restore-cancel-queued",
        )
        .await
        .expect("restore should be queued");
    let cancelled = persistence
        .cancel_restore(
            user.user_id,
            "user",
            project.id,
            queued.id,
            "no longer needed",
            "restore-cancel-request",
        )
        .await
        .expect("queued restore should cancel");
    assert_eq!(cancelled.status, "cancelled");
    assert!(!cancelled.cancel_requested);

    let running = persistence
        .enqueue_restore(
            user.user_id,
            "user",
            project.id,
            None,
            Some(0),
            "running cancellation",
            "restore-cancel-running",
        )
        .await
        .expect("second restore should be queued");
    sqlx::query(
        "UPDATE restore_jobs \
         SET run_after = CURRENT_TIMESTAMP + INTERVAL '1 hour', \
             lease_expires_at = CURRENT_TIMESTAMP + INTERVAL '1 hour' \
         WHERE id <> $1 AND status IN ('queued', 'running')",
    )
    .bind(running.id)
    .execute(persistence.pool())
    .await
    .expect("unrelated restore jobs should not race the focused test");
    let claimed = persistence
        .claim_restore_job()
        .await
        .expect("restore should be claimable")
        .expect("running restore should exist");
    assert_eq!(claimed.id, running.id);
    let lease = claimed
        .lease_token
        .expect("restore lease should be present");
    let requested = persistence
        .cancel_restore(
            user.user_id,
            "user",
            project.id,
            running.id,
            "stop restore",
            "restore-cancel-running-request",
        )
        .await
        .expect("running restore should accept cancellation");
    assert_eq!(requested.status, "running");
    assert!(requested.cancel_requested);
    persistence
        .cancel_restore_with_lease(running.id, lease)
        .await
        .expect("worker should finalize cancellation");
    let final_job = persistence
        .get_restore_job(user.user_id, "user", project.id, running.id)
        .await
        .expect("cancelled restore should remain queryable");
    assert_eq!(final_job.status, "cancelled");
    let project = persistence
        .get_project(user.user_id, "user", project.id)
        .await
        .expect("project should reopen after cancellation");
    assert_eq!(project.status, "active");
}

async fn assert_project_isolation(
    persistence: &Persistence,
    user_one: &ActivatedUser,
    user_two: &ActivatedUser,
) -> Uuid {
    let project_one = persistence
        .create_project(user_one.user_id, "user", "User one")
        .await
        .expect("first project should be created");
    let project_two = persistence
        .create_project(user_two.user_id, "user", "User two")
        .await
        .expect("second project should be created");

    let visible_projects = persistence
        .list_projects(user_one.user_id, "user")
        .await
        .expect("owned projects should be listed");
    assert_eq!(visible_projects.len(), 1);
    assert_eq!(visible_projects[0].id, project_one.id);
    assert!(matches!(
        persistence
            .get_project(user_one.user_id, "user", project_two.id)
            .await,
        Err(PersistenceError::NotFound)
    ));
    project_two.id
}

async fn assert_device_isolation(
    persistence: &Persistence,
    user_one: &ActivatedUser,
    user_two: &ActivatedUser,
) {
    persistence
        .register_device(
            user_one.user_id,
            "user",
            user_one.device_id,
            &DeviceProfile {
                display_name: "Device one".to_owned(),
                platform: "linux".to_owned(),
                app_version: "1.0.0".to_owned(),
            },
        )
        .await
        .expect("first device profile should update");
    let visible_devices = persistence
        .list_devices(user_one.user_id, "user")
        .await
        .expect("owned devices should be listed");
    assert_eq!(visible_devices.len(), 1);
    assert_eq!(visible_devices[0].id, user_one.device_id);
    assert!(matches!(
        persistence
            .update_device(user_one.user_id, "user", user_two.device_id, "Unauthorized")
            .await,
        Err(PersistenceError::NotFound)
    ));
}

async fn assert_admin_boundary(
    persistence: &Persistence,
    admin_id: Uuid,
    user_one: &ActivatedUser,
    user_two: &ActivatedUser,
    project_two_id: Uuid,
) {
    assert!(matches!(
        persistence.admin_list_users(user_one.user_id).await,
        Err(PersistenceError::AdminRequired)
    ));
    let admin_projects = persistence
        .admin_list_projects(admin_id, user_two.user_id)
        .await
        .expect("admin metadata view should be available");
    assert_eq!(admin_projects.len(), 1);
    assert_eq!(admin_projects[0].id, project_two_id);
    let admin_users = persistence
        .admin_list_users(admin_id)
        .await
        .expect("admin user metadata should be available");
    let serialized = serde_json::to_string(&admin_users).expect("metadata should serialize");
    assert!(!serialized.contains("passwordHash"));
    assert!(!serialized.contains("payload"));
    let activation_audits: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_events WHERE action = 'auth.invitation_activated' \
         AND request_id = 'activation-test'",
    )
    .fetch_one(persistence.pool())
    .await
    .expect("activation audit events should be queryable in the test owner session");
    assert!(activation_audits >= 2);
}

async fn assert_refresh_reuse_revokes_family(
    persistence: &Persistence,
    user_one: &ActivatedUser,
    test_id: Uuid,
) {
    let replacement_hash = token_hash(test_id, 31);
    let replacement = NewRefreshToken {
        token_hash: replacement_hash.clone(),
        family_id: Uuid::new_v4(),
        expires_at: refresh_expiry(3_600),
    };
    persistence
        .rotate_refresh_token(&user_one.refresh_hash, &replacement, "refresh-test")
        .await
        .expect("first refresh should rotate");
    assert!(matches!(
        persistence
            .rotate_refresh_token(&user_one.refresh_hash, &replacement, "reuse-test")
            .await,
        Err(PersistenceError::RefreshTokenReuse)
    ));
    assert!(matches!(
        persistence
            .rotate_refresh_token(&replacement_hash, &replacement, "reuse-test")
            .await,
        Err(PersistenceError::RefreshTokenReuse)
    ));
}

async fn assert_device_revocation(persistence: &Persistence, user_two: &ActivatedUser) {
    persistence
        .revoke_device(user_two.user_id, "user", user_two.device_id, "revoke-test")
        .await
        .expect("device should be revoked");
    assert!(matches!(
        persistence
            .validate_access(user_two.user_id, user_two.device_id, "user")
            .await,
        Err(PersistenceError::DeviceRevoked)
    ));
}

struct ActivatedUser {
    user_id: Uuid,
    device_id: Uuid,
    refresh_hash: Vec<u8>,
}

async fn activate_user(
    persistence: &Persistence,
    admin_id: Uuid,
    test_id: Uuid,
    ordinal: u8,
) -> ActivatedUser {
    let email = format!("user-{ordinal}-{test_id}@example.test");
    let invitation_hash = token_hash(test_id, ordinal);
    persistence
        .create_invitation(
            admin_id,
            &NewInvitation {
                email_normalized: email.clone(),
                email_display: email,
                token_hash: invitation_hash.clone(),
                expires_at: invitation_expiry(),
            },
            "integration-test",
        )
        .await
        .expect("invitation should be created");
    let device_id = Uuid::new_v4();
    let refresh_hash = token_hash(test_id, ordinal + 10);
    let session = persistence
        .activate_invitation(
            &invitation_hash,
            "test-password-hash",
            device_id,
            &NewRefreshToken {
                token_hash: refresh_hash.clone(),
                family_id: Uuid::new_v4(),
                expires_at: refresh_expiry(3_600),
            },
            "activation-test",
        )
        .await
        .expect("invitation should activate");
    assert!(session.user.last_login_at.is_some());
    let devices = persistence
        .list_devices(session.user.id, "user")
        .await
        .expect("activated device should be visible");
    assert_eq!(devices.len(), 1);
    assert!(devices[0].last_seen_at.is_some());
    assert!(devices[0].last_login_at.is_some());
    ActivatedUser {
        user_id: session.user.id,
        device_id,
        refresh_hash,
    }
}

fn token_hash(test_id: Uuid, discriminator: u8) -> Vec<u8> {
    let mut hash = Vec::with_capacity(17);
    hash.extend_from_slice(test_id.as_bytes());
    hash.push(discriminator);
    hash
}
