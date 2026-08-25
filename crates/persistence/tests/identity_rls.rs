use tasktips_persistence::{
    DeviceProfile, NewInvitation, NewRefreshToken, Persistence, PersistenceError,
    invitation_expiry, refresh_expiry,
};
use uuid::Uuid;

#[tokio::test]
async fn rls_and_auth_state_isolate_two_users() {
    let Ok(database_url) = std::env::var("TASKTIPS_DATABASE_URL") else {
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
