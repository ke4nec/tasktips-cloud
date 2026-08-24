use std::{env, net::SocketAddr};

use tasktips_api::{Readiness, build_router};
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
    let listener = TcpListener::bind(address).await?;
    info!(%address, "tasktips API listening");

    axum::serve(listener, build_router(Readiness::default())).await?;
    Ok(())
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .json()
        .init();
}
