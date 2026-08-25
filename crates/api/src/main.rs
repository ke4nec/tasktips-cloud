use std::{env, fs, net::SocketAddr};

use tasktips_api::{
    AppState, Readiness,
    auth::{AuthService, hash_password},
    build_application_router,
    cursor::CursorSigner,
};
use tasktips_application::{normalize_email, valid_password};
use tasktips_object_store::{ObjectStore, RustFsConfig};
use tasktips_persistence::Persistence;
use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_tracing();

    let arguments = env::args().skip(1).collect::<Vec<_>>();
    if arguments
        .first()
        .is_some_and(|argument| argument == "migrate")
    {
        let database_url = env::var("TASKTIPS_DATABASE_URL")?;
        let persistence = Persistence::connect(&database_url).await?;
        persistence.migrate().await?;
        info!("database migrations completed");
        return Ok(());
    }
    if arguments
        .first()
        .is_some_and(|argument| argument == "admin")
    {
        return create_admin(&arguments).await;
    }

    let address = env::var("TASKTIPS_BIND_ADDRESS")
        .unwrap_or_else(|_| "127.0.0.1:8080".to_owned())
        .parse::<SocketAddr>()?;
    let database = env::var("TASKTIPS_DATABASE_URL")
        .ok()
        .map(|url| Persistence::connect_lazy(&url))
        .transpose()?;
    let auth = database
        .as_ref()
        .map(|_| build_auth_service())
        .transpose()?;
    let object_store = build_object_store();
    let readiness = Readiness::new(database.clone(), object_store.clone());
    let mut state = AppState::new(readiness, database, auth);
    if let Some(object_store) = object_store {
        let cursor_secret = env::var("TASKTIPS_CURSOR_SIGNING_SECRET")?;
        state = state.with_sync(object_store, CursorSigner::new(cursor_secret)?);
    }
    let listener = TcpListener::bind(address).await?;
    info!(%address, "tasktips API listening");

    axum::serve(listener, build_application_router(state)).await?;
    Ok(())
}

async fn create_admin(arguments: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let email = match arguments {
        [admin, create, flag, email]
            if admin == "admin" && create == "create" && flag == "--email" =>
        {
            email
        }
        _ => return Err("用法: tasktips-api admin create --email <email>".into()),
    };
    let email_normalized = normalize_email(email).ok_or("邮箱格式无效")?;
    let password = rpassword::prompt_password("密码: ")?;
    if !valid_password(&password) {
        return Err("密码至少需要 12 个字符".into());
    }
    let confirmation = rpassword::prompt_password("再次输入密码: ")?;
    if password != confirmation {
        return Err("两次输入的密码不一致".into());
    }
    let password_hash = hash_password(&password)?;
    let database_url = env::var("TASKTIPS_DATABASE_URL")?;
    let persistence = Persistence::connect(&database_url).await?;
    let admin = persistence
        .create_initial_admin(&email_normalized, email.trim(), &password_hash)
        .await?;
    println!("已创建系统管理员 {} ({})", admin.email, admin.id);
    Ok(())
}

fn build_auth_service() -> Result<AuthService, Box<dyn std::error::Error>> {
    let key_path = env::var("TASKTIPS_JWT_PRIVATE_KEY_FILE")?;
    let private_key = fs::read(key_path)?;
    let access_ttl_seconds = env::var("TASKTIPS_ACCESS_TOKEN_TTL_SECONDS")
        .unwrap_or_else(|_| "900".to_owned())
        .parse()?;
    let refresh_ttl_seconds = env::var("TASKTIPS_REFRESH_TOKEN_TTL_SECONDS")
        .unwrap_or_else(|_| "2592000".to_owned())
        .parse()?;
    Ok(AuthService::from_private_key_pem(
        &private_key,
        env::var("TASKTIPS_JWT_ISSUER").unwrap_or_else(|_| "tasktips-cloud".to_owned()),
        env::var("TASKTIPS_JWT_AUDIENCE").unwrap_or_else(|_| "tasktips-desktop".to_owned()),
        access_ttl_seconds,
        refresh_ttl_seconds,
    )?)
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
