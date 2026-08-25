use std::{env, net::SocketAddr};

use tasktips_api::{Readiness, build_router};
use tasktips_object_store::{ObjectStore, RustFsConfig};
use tasktips_persistence::Persistence;
use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_tracing();

    if env::args().nth(1).as_deref() == Some("migrate") {
        let database_url = env::var("TASKTIPS_DATABASE_URL")?;
        let persistence = Persistence::connect(&database_url).await?;
        persistence.migrate().await?;
        info!("database migrations completed");
        return Ok(());
    }

    let address = env::var("TASKTIPS_BIND_ADDRESS")
        .unwrap_or_else(|_| "127.0.0.1:8080".to_owned())
        .parse::<SocketAddr>()?;
    let database = env::var("TASKTIPS_DATABASE_URL")
        .ok()
        .and_then(|url| Persistence::connect_lazy(&url).ok());
    let object_store = build_object_store();
    let readiness = Readiness::new(database, object_store);
    let listener = TcpListener::bind(address).await?;
    info!(%address, "tasktips API listening");

    axum::serve(listener, build_router(readiness)).await?;
    Ok(())
}

fn build_object_store() -> Option<ObjectStore> {
    let access_key = env::var("RUSTFS_ACCESS_KEY").ok()?;
    let secret_key = env::var("RUSTFS_SECRET_KEY").ok()?;
    let config = RustFsConfig::new(
        env::var("RUSTFS_ENDPOINT").unwrap_or_else(|_| "http://127.0.0.1:9000".to_owned()),
        env::var("RUSTFS_REGION").unwrap_or_else(|_| "us-east-1".to_owned()),
        env::var("RUSTFS_BUCKET").unwrap_or_else(|_| "tasktips-data".to_owned()),
    );
    Some(ObjectStore::with_credentials(
        config, access_key, secret_key,
    ))
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .json()
        .init();
}
