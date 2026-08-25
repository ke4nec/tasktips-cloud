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

    info!("tasktips worker started");
    loop {
        tokio::select! {
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
