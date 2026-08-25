use aws_sdk_s3::{Client as S3Client, primitives::ByteStream};
use bytes::Bytes;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RustFsConfig {
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub force_path_style: bool,
}

impl RustFsConfig {
    #[must_use]
    pub fn new(endpoint: String, region: String, bucket: String) -> Self {
        Self {
            endpoint,
            region,
            bucket,
            force_path_style: true,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ObjectStoreError {
    #[error("payload was not found")]
    NotFound,
    #[error("payload content hash did not match")]
    HashMismatch,
    #[error("object store operation failed")]
    Backend,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PayloadInfo {
    pub bucket: String,
    pub key: String,
    pub content_hash: String,
    pub size: u64,
    pub media_type: String,
}

pub struct PayloadDownload {
    pub body: ByteStream,
    pub content_length: Option<i64>,
    pub content_range: Option<String>,
    pub media_type: String,
}

#[derive(Clone)]
pub struct ObjectStore {
    client: S3Client,
    bucket: String,
}

impl ObjectStore {
    #[must_use]
    pub fn with_credentials(
        config: RustFsConfig,
        access_key: impl Into<String>,
        secret_key: impl Into<String>,
    ) -> Self {
        let credentials = aws_sdk_s3::config::Credentials::new(
            access_key,
            secret_key,
            None,
            None,
            "tasktips-cloud",
        );
        let sdk_config = aws_sdk_s3::Config::builder()
            .behavior_version_latest()
            .endpoint_url(config.endpoint)
            .region(aws_sdk_s3::config::Region::new(config.region))
            .credentials_provider(credentials)
            .force_path_style(config.force_path_style)
            .build();

        Self {
            client: S3Client::from_conf(sdk_config),
            bucket: config.bucket,
        }
    }

    /// Checks the bucket and creates it on first use for a fresh `RustFS` volume.
    pub async fn is_ready(&self) -> bool {
        if self
            .client
            .head_bucket()
            .bucket(&self.bucket)
            .send()
            .await
            .is_ok()
        {
            return true;
        }

        self.client
            .create_bucket()
            .bucket(&self.bucket)
            .send()
            .await
            .is_ok()
    }

    /// Writes verified bytes to an owner/project-scoped immutable content-addressed key.
    ///
    /// # Errors
    ///
    /// Returns a hash mismatch before writing, or a generic backend error without credentials or
    /// internal object keys.
    pub async fn put_payload(
        &self,
        owner_user_id: Uuid,
        project_id: Uuid,
        expected_hash: &str,
        media_type: &str,
        bytes: Bytes,
    ) -> Result<PayloadInfo, ObjectStoreError> {
        let actual_hash = hex::encode(Sha256::digest(&bytes));
        if actual_hash != expected_hash {
            return Err(ObjectStoreError::HashMismatch);
        }
        let final_key = payload_key(owner_user_id, project_id, expected_hash);
        if let Some(existing) = self.head_key(&final_key).await? {
            return Ok(PayloadInfo {
                bucket: self.bucket.clone(),
                key: final_key,
                content_hash: expected_hash.to_owned(),
                size: existing.0,
                media_type: existing.1.unwrap_or_else(|| media_type.to_owned()),
            });
        }

        let temporary_key = format!(
            "users/{owner_user_id}/projects/{project_id}/uploads/{}",
            Uuid::new_v4()
        );
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(&temporary_key)
            .content_length(i64::try_from(bytes.len()).map_err(|_| ObjectStoreError::Backend)?)
            .content_type(media_type)
            .body(ByteStream::from(bytes.clone()))
            .send()
            .await
            .map_err(|_| ObjectStoreError::Backend)?;

        let copy_result = self
            .client
            .copy_object()
            .bucket(&self.bucket)
            .key(&final_key)
            .copy_source(format!("{}/{}", self.bucket, temporary_key))
            .content_type(media_type)
            .metadata_directive(aws_sdk_s3::types::MetadataDirective::Replace)
            .send()
            .await;
        let _ = self
            .client
            .delete_object()
            .bucket(&self.bucket)
            .key(&temporary_key)
            .send()
            .await;
        copy_result.map_err(|_| ObjectStoreError::Backend)?;

        let (size, stored_media_type) = self
            .head_key(&final_key)
            .await?
            .ok_or(ObjectStoreError::Backend)?;
        if size != u64::try_from(bytes.len()).map_err(|_| ObjectStoreError::Backend)? {
            return Err(ObjectStoreError::Backend);
        }
        Ok(PayloadInfo {
            bucket: self.bucket.clone(),
            key: final_key,
            content_hash: expected_hash.to_owned(),
            size,
            media_type: stored_media_type.unwrap_or_else(|| media_type.to_owned()),
        })
    }

    /// Writes a verified, immutable snapshot manifest. A manifest contains metadata references
    /// only; payload bytes remain in their existing content-addressed objects.
    ///
    /// # Errors
    ///
    /// Returns a hash mismatch before writing or a backend error without exposing object keys.
    pub async fn put_manifest(
        &self,
        owner_user_id: Uuid,
        project_id: Uuid,
        snapshot_id: Uuid,
        expected_hash: &str,
        bytes: Bytes,
    ) -> Result<PayloadInfo, ObjectStoreError> {
        let actual_hash = hex::encode(Sha256::digest(&bytes));
        if actual_hash != expected_hash {
            return Err(ObjectStoreError::HashMismatch);
        }
        let key = manifest_key(owner_user_id, project_id, snapshot_id);
        if let Some(existing) = self.head_key(&key).await? {
            if existing.0 != u64::try_from(bytes.len()).map_err(|_| ObjectStoreError::Backend)? {
                return Err(ObjectStoreError::Backend);
            }
            return Ok(PayloadInfo {
                bucket: self.bucket.clone(),
                key,
                content_hash: expected_hash.to_owned(),
                size: existing.0,
                media_type: existing.1.unwrap_or_else(|| "application/json".to_owned()),
            });
        }
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(&key)
            .content_length(i64::try_from(bytes.len()).map_err(|_| ObjectStoreError::Backend)?)
            .content_type("application/json")
            .body(ByteStream::from(bytes))
            .send()
            .await
            .map_err(|_| ObjectStoreError::Backend)?;
        let (size, media_type) = self
            .head_key(&key)
            .await?
            .ok_or(ObjectStoreError::Backend)?;
        Ok(PayloadInfo {
            bucket: self.bucket.clone(),
            key,
            content_hash: expected_hash.to_owned(),
            size,
            media_type: media_type.unwrap_or_else(|| "application/json".to_owned()),
        })
    }

    /// # Errors
    ///
    /// Returns a backend error when `RustFS` cannot answer the metadata request.
    pub async fn head_payload(
        &self,
        owner_user_id: Uuid,
        project_id: Uuid,
        content_hash: &str,
    ) -> Result<Option<PayloadInfo>, ObjectStoreError> {
        let key = payload_key(owner_user_id, project_id, content_hash);
        Ok(self
            .head_key(&key)
            .await?
            .map(|(size, media_type)| PayloadInfo {
                bucket: self.bucket.clone(),
                key,
                content_hash: content_hash.to_owned(),
                size,
                media_type: media_type.unwrap_or_else(|| "application/octet-stream".to_owned()),
            }))
    }

    /// # Errors
    ///
    /// Returns not-found or backend errors without exposing the internal object key.
    pub async fn get_payload(
        &self,
        owner_user_id: Uuid,
        project_id: Uuid,
        content_hash: &str,
        range: Option<&str>,
    ) -> Result<PayloadDownload, ObjectStoreError> {
        let mut request = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(payload_key(owner_user_id, project_id, content_hash));
        if let Some(range) = range {
            request = request.range(range);
        }
        let output = request.send().await.map_err(|error| {
            if error
                .as_service_error()
                .is_some_and(aws_sdk_s3::operation::get_object::GetObjectError::is_no_such_key)
            {
                ObjectStoreError::NotFound
            } else {
                ObjectStoreError::Backend
            }
        })?;
        Ok(PayloadDownload {
            body: output.body,
            content_length: output.content_length,
            content_range: output.content_range,
            media_type: output
                .content_type
                .unwrap_or_else(|| "application/octet-stream".to_owned()),
        })
    }

    /// Deletes stale temporary or content-addressed objects that have no database reference.
    ///
    /// # Errors
    ///
    /// Returns a backend error if listing or deletion fails. Database references are supplied by
    /// the worker and are never deleted by this operation.
    pub async fn delete_orphans(
        &self,
        referenced_keys: &HashSet<String>,
        older_than: OffsetDateTime,
    ) -> Result<u64, ObjectStoreError> {
        let mut continuation_token = None;
        let mut deleted = 0_u64;
        loop {
            let output = self
                .client
                .list_objects_v2()
                .bucket(&self.bucket)
                .prefix("users/")
                .set_continuation_token(continuation_token)
                .send()
                .await
                .map_err(|_| ObjectStoreError::Backend)?;
            for object in output.contents() {
                let Some(key) = object.key() else {
                    continue;
                };
                let managed_object = key.contains("/payloads/sha256/")
                    || key.contains("/uploads/")
                    || key.contains("/snapshots/");
                let old_enough = object
                    .last_modified()
                    .and_then(|modified| OffsetDateTime::from_unix_timestamp(modified.secs()).ok())
                    .is_some_and(|modified| modified < older_than);
                if managed_object && old_enough && !referenced_keys.contains(key) {
                    self.client
                        .delete_object()
                        .bucket(&self.bucket)
                        .key(key)
                        .send()
                        .await
                        .map_err(|_| ObjectStoreError::Backend)?;
                    deleted += 1;
                }
            }
            continuation_token = output.next_continuation_token().map(str::to_owned);
            if continuation_token.is_none() {
                break;
            }
        }
        Ok(deleted)
    }

    async fn head_key(&self, key: &str) -> Result<Option<(u64, Option<String>)>, ObjectStoreError> {
        match self
            .client
            .head_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
        {
            Ok(output) => Ok(Some((
                u64::try_from(output.content_length.unwrap_or_default())
                    .map_err(|_| ObjectStoreError::Backend)?,
                output.content_type,
            ))),
            Err(error)
                if error.as_service_error().is_some_and(
                    aws_sdk_s3::operation::head_object::HeadObjectError::is_not_found,
                ) =>
            {
                Ok(None)
            }
            Err(_) => Err(ObjectStoreError::Backend),
        }
    }
}

#[must_use]
pub fn payload_key(owner_user_id: Uuid, project_id: Uuid, content_hash: &str) -> String {
    let shard = content_hash.get(..2).unwrap_or(content_hash);
    format!("users/{owner_user_id}/projects/{project_id}/payloads/sha256/{shard}/{content_hash}")
}

#[must_use]
pub fn manifest_key(owner_user_id: Uuid, project_id: Uuid, snapshot_id: Uuid) -> String {
    format!("users/{owner_user_id}/projects/{project_id}/snapshots/{snapshot_id}/manifest.json")
}

#[cfg(test)]
mod tests {
    use super::payload_key;
    use uuid::Uuid;

    #[test]
    fn payload_keys_are_scoped_and_content_addressed() {
        let owner = Uuid::nil();
        let project = Uuid::from_u128(1);
        let hash = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
        assert_eq!(
            payload_key(owner, project, hash),
            format!("users/{owner}/projects/{project}/payloads/sha256/ab/{hash}")
        );
    }
}
