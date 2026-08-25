use bytes::Bytes;
use sha2::{Digest, Sha256};
use std::{env, time::Duration};
use tasktips_object_store::{ObjectStore, RustFsConfig, parse_payload_key};
use tasktips_persistence::Persistence;
use time::OffsetDateTime;
use tokio::time::{MissedTickBehavior, interval};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

const ORPHAN_AGE: time::Duration = time::Duration::hours(24);

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .json()
        .init();

    let persistence = Persistence::connect(&env::var("TASKTIPS_DATABASE_URL")?).await?;
    let object_store = ObjectStore::with_credentials(
        RustFsConfig::new(
            env::var("RUSTFS_ENDPOINT")?,
            env::var("RUSTFS_REGION").unwrap_or_else(|_| "us-east-1".to_owned()),
            env::var("RUSTFS_BUCKET")?,
        ),
        env::var("RUSTFS_ACCESS_KEY")?,
        env::var("RUSTFS_SECRET_KEY")?,
    );
    let interval_seconds = env::var("TASKTIPS_ORPHAN_CLEANUP_INTERVAL_SECONDS")
        .unwrap_or_else(|_| "3600".to_owned())
        .parse::<u64>()?;
    let mut cleanup_interval = interval(Duration::from_secs(interval_seconds.max(60)));
    cleanup_interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut restore_interval = interval(Duration::from_secs(5));
    restore_interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

    info!("tasktips worker started");
    loop {
        tokio::select! {
            _ = restore_interval.tick() => {
                if let Err(error) = process_restore_jobs(&persistence, &object_store).await {
                    warn!(error = %error, "restore job processing failed");
                }
            }
            _ = cleanup_interval.tick() => {
                if let Err(error) = cleanup_orphans(&persistence, &object_store).await {
                    warn!(error = %error, "payload orphan cleanup failed");
                }
            }
            signal = tokio::signal::ctrl_c() => {
                signal?;
                break;
            }
        }
    }
    info!("tasktips worker stopped");
    Ok(())
}

async fn process_restore_jobs(
    persistence: &Persistence,
    object_store: &ObjectStore,
) -> Result<(), Box<dyn std::error::Error>> {
    while let Some(job) = persistence.claim_restore_job().await? {
        let lease_token = job.lease_token.ok_or("restore claim missing lease token")?;
        let prepared = persistence.create_pre_restore_snapshot(&job).await;
        let (snapshot_id, owner_user_id, bytes) = match prepared {
            Ok(value) => value,
            Err(error) => {
                persistence
                    .fail_restore_job_with_lease(job.id, lease_token, "PRE_RESTORE_SNAPSHOT_FAILED")
                    .await?;
                return Err(error.into());
            }
        };
        let hash = hex::encode(Sha256::digest(&bytes));
        persistence.renew_restore_lease(job.id, lease_token).await?;
        let manifest = object_store
            .put_manifest(
                owner_user_id,
                job.project_id,
                snapshot_id,
                &hash,
                Bytes::from(bytes),
            )
            .await;
        let manifest = match manifest {
            Ok(value) => value,
            Err(error) => {
                persistence
                    .fail_restore_job_with_lease(job.id, lease_token, "PRE_RESTORE_MANIFEST_FAILED")
                    .await?;
                return Err(error.into());
            }
        };
        persistence
            .mark_snapshot_ready_for_job(
                job.id,
                lease_token,
                snapshot_id,
                &manifest.bucket,
                &manifest.key,
            )
            .await?;
        persistence.renew_restore_lease(job.id, lease_token).await?;
        if let Err(error) = persistence
            .execute_restore_with_lease(job.id, lease_token)
            .await
        {
            persistence
                .fail_restore_job_with_lease(job.id, lease_token, "RESTORE_FAILED")
                .await?;
            return Err(error.into());
        }
    }
    Ok(())
}

async fn cleanup_orphans(
    persistence: &Persistence,
    object_store: &ObjectStore,
) -> Result<(), Box<dyn std::error::Error>> {
    let cutoff = OffsetDateTime::now_utc() - ORPHAN_AGE;
    let candidates = persistence.unreferenced_payload_keys(cutoff).await?;
    let mut catalog_entries = 0_u64;
    for (project_id, content_hash) in candidates {
        let mut lock = persistence
            .acquire_payload_lock(project_id, &content_hash)
            .await?;
        let Some(object_key) = lock.unreferenced_payload_key(cutoff).await? else {
            lock.finish().await?;
            continue;
        };
        if let Err(error) = object_store.delete_key(&object_key).await {
            lock.rollback().await;
            return Err(error.into());
        }
        lock.delete_payload_row().await?;
        catalog_entries += 1;
    }
    let referenced_keys = persistence.referenced_payload_keys().await?;
    let stale_payload_keys = object_store
        .stale_payload_keys(&referenced_keys, cutoff)
        .await?;
    let mut deleted_payload_objects = 0_u64;
    for object_key in stale_payload_keys {
        let Some((_, project_id, content_hash)) = parse_payload_key(&object_key) else {
            continue;
        };
        let mut lock = persistence
            .acquire_payload_lock(project_id, &content_hash)
            .await?;
        if lock.payload_is_referenced().await? {
            lock.finish().await?;
            continue;
        }
        if let Err(error) = object_store.delete_key(&object_key).await {
            lock.rollback().await;
            return Err(error.into());
        }
        lock.finish().await?;
        deleted_payload_objects += 1;
    }
    let referenced_keys = persistence.referenced_payload_keys().await?;
    let deleted_objects = object_store
        .delete_orphans(&referenced_keys, cutoff)
        .await?;
    info!(
        catalog_entries,
        deleted_objects = deleted_objects + deleted_payload_objects,
        "payload orphan cleanup completed"
    );
    Ok(())
}
