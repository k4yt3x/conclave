use conclave_server::{api, config, db, state};

use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use tracing::info;

#[derive(Parser)]
#[command(name = "conclave-server", about = "Conclave E2EE messaging server")]
struct Cli {
    /// Path to the server configuration file.
    #[arg(short, long, default_value = "conclave-server.toml")]
    config: PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "conclave_server=info".into()),
        )
        .init();

    let cli = Cli::parse();

    let config: config::ServerConfig = if cli.config.exists() {
        let contents = std::fs::read_to_string(&cli.config)?;
        toml::from_str(&contents)?
    } else {
        info!(
            "config file {} not found, using defaults",
            cli.config.display()
        );
        config::ServerConfig::default()
    };

    info!("opening database at {}", config.database_path.display());
    let database = db::Database::open(&config.database_path)?;

    let bind_address = config.bind_address.clone();
    let tls_cert_path = config.tls_cert_path.clone();
    let tls_key_path = config.tls_key_path.clone();
    let app_state = Arc::new(state::AppState::new(database, config));

    let app = api::router().with_state(app_state.clone());

    // Periodically clean up expired sessions.
    {
        let state = app_state.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
            loop {
                interval.tick().await;
                match state.db.cleanup_expired_sessions() {
                    Ok(count) if count > 0 => {
                        info!("cleaned up {count} expired session(s)");
                    }
                    _ => {}
                }
            }
        });
    }

    match (&tls_cert_path, &tls_key_path) {
        (Some(cert_path), Some(key_path)) => {
            let tls_config =
                axum_server::tls_rustls::RustlsConfig::from_pem_file(cert_path, key_path).await?;
            let addr: std::net::SocketAddr = bind_address.parse()?;
            info!("listening on {bind_address} (TLS)");
            axum_server::bind_rustls(addr, tls_config)
                .serve(app.into_make_service())
                .await?;
        }
        (None, None) => {
            let listener = tokio::net::TcpListener::bind(&bind_address).await?;
            info!("listening on {bind_address} (plain HTTP)");
            axum::serve(listener, app).await?;
        }
        _ => {
            anyhow::bail!(
                "both tls_cert_path and tls_key_path must be set for TLS, or neither for plain HTTP"
            );
        }
    }

    Ok(())
}
