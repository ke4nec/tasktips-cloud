use bytes::Bytes;
use sha2::{Digest, Sha256};
use std::{env, time::Duration};
use tasktips_object_store::{ObjectStore, RustFsConfig};
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
        let prepared = persistence.create_pre_restore_snapshot(&job).await;
        let (snapshot_id, owner_user_id, bytes) = match prepared {
            Ok(value) => value,
            Err(error) => {
                persistence
                    .fail_restore_job(job.id, "PRE_RESTORE_SNAPSHOT_FAILED")
                    .await?;
                return Err(error.into());
            }
        };
        let hash = hex::encode(Sha256::digest(&bytes));
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
                    .fail_restore_job(job.id, "PRE_RESTORE_MANIFEST_FAILED")
                    .await?;
                return Err(error.into());
            }
        };
        persistence
            .mark_snapshot_ready(snapshot_id, &manifest.bucket, &manifest.key)
            .await?;
        if let Err(error) = persistence.execute_restore(job.id).await {
            persistence
                .fail_restore_job(job.id, "RESTORE_FAILED")
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
    let catalog_entries = persistence.prune_unreferenced_payloads(cutoff).await?;
    let referenced_keys = persistence.referenced_payload_keys().await?;
    let deleted_objects = object_store
        .delete_orphans(&referenced_keys, cutoff)
        .await?;
    info!(
        catalog_entries = catalog_entries.len(),
        deleted_objects, "payload orphan cleanup completed"
    );
    Ok(())
}
