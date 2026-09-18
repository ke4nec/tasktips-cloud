use bytes::Bytes;
use sha2::{Digest, Sha256};
use std::{
    env,
    path::{Path, PathBuf},
    time::Duration,
};
use tasktips_object_store::{ObjectStore, RustFsConfig, parse_payload_key};
use tasktips_persistence::Persistence;
use time::OffsetDateTime;
use tokio::{
    fs,
    io::{AsyncReadExt, AsyncWriteExt, copy},
    time::{MissedTickBehavior, interval},
};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

const ORPHAN_AGE: time::Duration = time::Duration::hours(24);
const DEFAULT_EXPORT_TMP_MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .json()
        .init();

    if env::args().any(|argument| argument == "--version" || argument == "-V") {
        println!("tasktips-worker {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
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
    cleanup_export_temp_dir().await;

    info!(
        version = env!("CARGO_PKG_VERSION"),
        "tasktips worker started"
    );
    loop {
        tokio::select! {
            _ = restore_interval.tick() => {
                if let Err(error) = process_restore_jobs(&persistence, &object_store).await {
                    warn!(error = %error, "restore job processing failed");
                }
                if let Err(error) = process_purge_jobs(&persistence, &object_store).await {
                    warn!(error = %error, "purge job processing failed");
                }
            }
            _ = cleanup_interval.tick() => {
                if let Err(error) = cleanup_orphans(&persistence, &object_store).await {
                    warn!(error = %error, "payload orphan cleanup failed");
                }
                if let Err(error) = persistence.prune_distributed_rate_limits(10_000).await {
                    warn!(error = %error, "distributed rate-limit cleanup failed");
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
            Err(tasktips_persistence::PersistenceError::RestoreCancelled) => {
                persistence
                    .cancel_restore_with_lease(job.id, lease_token)
                    .await?;
                continue;
            }
            Err(error) => {
                persistence
                    .retry_restore_job_with_lease(
                        job.id,
                        lease_token,
                        "PRE_RESTORE_SNAPSHOT_FAILED",
                    )
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
                    .retry_restore_job_with_lease(
                        job.id,
                        lease_token,
                        "PRE_RESTORE_MANIFEST_FAILED",
                    )
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
            if matches!(
                &error,
                tasktips_persistence::PersistenceError::RestoreCancelled
            ) {
                persistence
                    .cancel_restore_with_lease(job.id, lease_token)
                    .await?;
                continue;
            }
            persistence
                .retry_restore_job_with_lease(job.id, lease_token, "RESTORE_FAILED")
                .await?;
            return Err(error.into());
        }
    }
    Ok(())
}

async fn process_purge_jobs(
    persistence: &Persistence,
    object_store: &ObjectStore,
) -> Result<(), Box<dyn std::error::Error>> {
    while let Some(job) = persistence.claim_account_purge_job().await? {
        let lease_token = job
            .lease_token
            .ok_or("account purge claim missing lease token")?;
        let result =
            process_account_purge_job(persistence, object_store, job.id, lease_token).await;
        if let Err(error) = result {
            persistence
                .fail_account_purge(job.id, lease_token, "ACCOUNT_PURGE_FAILED")
                .await?;
            return Err(error);
        }
    }
    while let Some(job) = persistence.claim_project_purge_job().await? {
        let lease_token = job.lease_token.ok_or("purge claim missing lease token")?;
        let keys = match persistence.prepare_project_purge(job.id, lease_token).await {
            Ok(keys) => keys,
            Err(error) => {
                persistence
                    .fail_project_purge(job.id, lease_token, "PURGE_DETACH_FAILED")
                    .await?;
                return Err(error.into());
            }
        };
        for key in &keys {
            if let Err(error) = object_store.delete_key(key).await {
                persistence
                    .fail_project_purge(job.id, lease_token, "PURGE_OBJECT_DELETE_FAILED")
                    .await?;
                return Err(error.into());
            }
        }
        persistence
            .complete_project_purge(job.id, lease_token)
            .await?;
    }
    Ok(())
}

async fn process_account_purge_job(
    persistence: &Persistence,
    object_store: &ObjectStore,
    job_id: uuid::Uuid,
    lease_token: uuid::Uuid,
) -> Result<(), Box<dyn std::error::Error>> {
    let phase = persistence.account_purge_phase(job_id, lease_token).await?;
    if phase == "export" {
        let export = persistence
            .prepare_account_export(job_id, lease_token)
            .await?;
        let base = PathBuf::from(
            env::var("TASKTIPS_EXPORT_TMP_DIR")
                .unwrap_or_else(|_| "/tmp/tasktips-exports".to_owned()),
        );
        let work_dir = base.join(export.export_id.to_string());
        fs::create_dir_all(&work_dir).await?;
        let result = async {
            let mut files = Vec::with_capacity(export.payloads.len());
            let max_temp_bytes = env::var("TASKTIPS_EXPORT_TMP_MAX_BYTES")
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .filter(|value| *value >= 1024 * 1024)
                .unwrap_or(DEFAULT_EXPORT_TMP_MAX_BYTES);
            let mut temporary_bytes = u64::try_from(export.manifest.len())?;
            for (index, payload) in export.payloads.iter().enumerate() {
                persistence
                    .renew_account_purge_lease(job_id, lease_token)
                    .await?;
                let download = object_store.get_key(&payload.object_key).await?;
                let payload_size = download
                    .content_length
                    .and_then(|size| u64::try_from(size).ok())
                    .ok_or("export payload size unavailable")?;
                temporary_bytes = temporary_bytes
                    .checked_add(payload_size)
                    .ok_or("export temporary size overflow")?;
                if temporary_bytes > max_temp_bytes {
                    return Err("export temporary capacity exceeded".into());
                }
                let path = work_dir.join(format!("payload-{index}"));
                let mut reader = download.body.into_async_read();
                let mut file = create_private_file(&path).await?;
                copy(&mut reader, &mut file).await?;
                file.flush().await?;
                files.push((path, payload.archive_path.clone()));
            }
            let archive_path = work_dir.join("export.tar.zst");
            let archive_output = archive_path.clone();
            let manifest = export.manifest.clone();
            let archive_files = files.clone();
            tokio::task::spawn_blocking(move || {
                build_export_archive(&archive_output, &manifest, &archive_files)
            })
            .await??;
            let archive_size = fs::metadata(&archive_path).await?.len();
            if temporary_bytes
                .checked_add(archive_size)
                .is_none_or(|size| size > max_temp_bytes)
            {
                return Err("export temporary capacity exceeded".into());
            }
            let (hash, size) = hash_file(&archive_path).await?;
            object_store
                .put_export_file(
                    export.owner_user_id,
                    export.export_id,
                    &hash,
                    &archive_path,
                    size,
                )
                .await?;
            persistence
                .complete_account_export(job_id, lease_token, export.export_id, &hash, size)
                .await?;
            Ok::<(), Box<dyn std::error::Error>>(())
        }
        .await;
        let _ = fs::remove_dir_all(&work_dir).await;
        result
    } else {
        let keys = persistence
            .prepare_account_purge(job_id, lease_token)
            .await?;
        for key in keys {
            object_store.delete_key(&key).await?;
        }
        persistence
            .complete_account_purge(job_id, lease_token)
            .await?;
        Ok(())
    }
}

async fn create_private_file(path: &Path) -> Result<fs::File, std::io::Error> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    options.mode(0o600);
    options.open(path).await
}

async fn cleanup_export_temp_dir() {
    let base = PathBuf::from(
        env::var("TASKTIPS_EXPORT_TMP_DIR").unwrap_or_else(|_| "/tmp/tasktips-exports".to_owned()),
    );
    let Ok(mut entries) = fs::read_dir(&base).await else {
        return;
    };
    let cutoff = std::time::SystemTime::now()
        .checked_sub(Duration::from_hours(24))
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
    while let Ok(Some(entry)) = entries.next_entry().await {
        let Ok(metadata) = entry.metadata().await else {
            continue;
        };
        let stale = metadata
            .modified()
            .ok()
            .is_some_and(|modified| modified < cutoff);
        if stale {
            let path = entry.path();
            let result = if metadata.is_dir() {
                fs::remove_dir_all(path).await
            } else {
                fs::remove_file(path).await
            };
            if let Err(error) = result {
                warn!(%error, "stale export temporary file cleanup failed");
            }
        }
    }
}

async fn hash_file(path: &Path) -> Result<(String, u64), Box<dyn std::error::Error>> {
    let mut file = fs::File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut size = 0_u64;
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        size = size
            .checked_add(u64::try_from(read)?)
            .ok_or("archive size overflow")?;
        hasher.update(&buffer[..read]);
    }
    Ok((hex::encode(hasher.finalize()), size))
}

fn build_export_archive(
    output: &Path,
    manifest: &[u8],
    files: &[(PathBuf, String)],
) -> Result<(), std::io::Error> {
    let mut output_options = std::fs::OpenOptions::new();
    output_options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut output_options, 0o600);
    let output_file = output_options.open(output)?;
    let encoder = zstd::Encoder::new(output_file, 3).map_err(std::io::Error::other)?;
    let mut archive = tar::Builder::new(encoder);
    let mut header = tar::Header::new_gnu();
    header.set_size(u64::try_from(manifest.len()).map_err(std::io::Error::other)?);
    header.set_mode(0o600);
    header.set_mtime(0);
    header.set_cksum();
    archive
        .append_data(&mut header, "manifest.json", manifest)
        .map_err(std::io::Error::other)?;
    for (path, archive_path) in files {
        archive
            .append_path_with_name(path, archive_path)
            .map_err(std::io::Error::other)?;
    }
    let encoder = archive.into_inner().map_err(std::io::Error::other)?;
    encoder
        .finish()
        .map_err(std::io::Error::other)?
        .sync_all()?;
    Ok(())
}

async fn cleanup_orphans(
    persistence: &Persistence,
    object_store: &ObjectStore,
) -> Result<(), Box<dyn std::error::Error>> {
    let cutoff = OffsetDateTime::now_utc() - ORPHAN_AGE;
    let expired_snapshot_keys = persistence.expire_pending_snapshots(cutoff).await?;
    for key in expired_snapshot_keys {
        object_store.delete_key(&key).await?;
    }
    let expired_bootstrap_keys = persistence.expire_bootstrap_manifests(1_000).await?;
    for key in expired_bootstrap_keys {
        object_store.delete_key(&key).await?;
    }
    let expired_account_export_keys = persistence.expire_account_purge_exports(1_000).await?;
    for key in expired_account_export_keys {
        object_store.delete_key(&key).await?;
    }
    let expired_idempotency = persistence.purge_expired_idempotency_records(1_000).await?;
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
        expired_idempotency,
        deleted_objects = deleted_objects + deleted_payload_objects,
        "payload orphan cleanup completed"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::build_export_archive;
    use std::{fs, io::Read, path::PathBuf};

    #[test]
    fn account_export_archive_contains_manifest_and_payload() {
        let suffix = uuid::Uuid::new_v4();
        let root = std::env::temp_dir().join(format!("tasktips-export-test-{suffix}"));
        fs::create_dir_all(&root).expect("test directory should be created");
        let payload = root.join("payload");
        fs::write(&payload, b"payload bytes").expect("payload should be written");
        let archive = root.join("export.tar.zst");
        build_export_archive(
            &archive,
            br#"{"format":"tasktips-account-export-v1"}"#,
            &[(payload, "payloads/project/hash".to_owned())],
        )
        .expect("archive should be created");

        let file = fs::File::open(&archive).expect("archive should be readable");
        let decoder = zstd::Decoder::new(file).expect("archive should be zstd");
        let mut tar = tar::Archive::new(decoder);
        let mut names = Vec::new();
        let mut manifest = String::new();
        for entry in tar.entries().expect("tar entries should be readable") {
            let mut entry = entry.expect("tar entry should be valid");
            names.push(
                entry
                    .path()
                    .expect("entry path should be valid")
                    .to_path_buf(),
            );
            if names.last() == Some(&PathBuf::from("manifest.json")) {
                entry
                    .read_to_string(&mut manifest)
                    .expect("manifest should be readable");
            }
        }
        assert!(
            names
                .iter()
                .any(|name| name == &PathBuf::from("manifest.json"))
        );
        assert!(
            names
                .iter()
                .any(|name| name == &PathBuf::from("payloads/project/hash"))
        );
        assert!(manifest.contains("tasktips-account-export-v1"));
        fs::remove_dir_all(root).expect("test directory should be removed");
    }
}
