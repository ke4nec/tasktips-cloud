use tasktips_persistence::Persistence;
use uuid::Uuid;

/// Regression test for migrations/0002: `admin_enqueue_restore` reads
/// `restore_jobs` (active-job check) as the `tasktips_admin` role before its
/// INSERT, so the role needs SELECT as well as INSERT.
#[tokio::test]
async fn admin_can_enqueue_restore_against_active_project() {
    let Ok(database_url) = std::env::var("TASKTIPS_DATABASE_URL") else {
        assert!(
            std::env::var_os("CI").is_none(),
            "TASKTIPS_DATABASE_URL is required for admin_restore_grants in CI"
        );
        eprintln!("admin_restore_grants skipped: TASKTIPS_DATABASE_URL is not set");
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
    let user = persistence
        .admin_create_user(
            admin.id,
            &format!("user-{test_id}@example.test"),
            &format!("user-{test_id}@example.test"),
            "test-password-hash",
            "req_restore_grants_user",
        )
        .await
        .expect("user should be created");
    let project = persistence
        .create_project(user.id, "user", "Restore grants project")
        .await
        .expect("project should be created");

    let job = persistence
        .admin_enqueue_restore(
            admin.id,
            project.id,
            None,
            Some(project.change_sequence),
            "regression: enqueue must pass the active-job check",
            "req_restore_grants_enqueue",
        )
        .await
        .expect("admin restore enqueue should not fail on role grants");
    assert_eq!(job.status, "queued");
    assert_eq!(job.project_id, project.id);
}
