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
