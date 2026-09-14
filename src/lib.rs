mod app;
mod error;
mod export;
mod graphql;
mod models;
mod naming;
mod probe;
mod service;
mod store;
mod subscription;

pub async fn run() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .compact()
        .init();

    let db_path = std::env::var("HONK_SUBSCRIBE_DB").unwrap_or_else(|_| "honk-subsribe.db".into());
    let bind = std::env::var("HONK_SUBSCRIBE_ADDR").unwrap_or_else(|_| "0.0.0.0:8090".into());
    let state = app::AppState::new(&db_path)?;
    let router = app::router(state);
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    tracing::info!(%bind, %db_path, "honk node control listening");
    axum::serve(listener, router).await?;
    Ok(())
}
