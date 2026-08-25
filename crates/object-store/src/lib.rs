pub use aws_sdk_s3::Client as S3Client;

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
}
