use std::{fs, sync::Arc};

use anyhow::Result;
use axum::middleware;
use axum_jwt_auth::LocalDecoder;

use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use rmcp::transport::{
    StreamableHttpServerConfig,
    streamable_http_server::{session::local::LocalSessionManager, tower::StreamableHttpService},
};
mod common;
mod gateway;
mod layers;
#[cfg(feature = "with_tools")]
mod tools;
mod user_config_store;
use gateway::McpService;
use layers::session_id::SessionId;

use tower_http::cors::{Any, CorsLayer};
use tracing::info;

use crate::{
    common::{MCP_AUDIENCE, McpGatewayAppState, RedisClient, RedisConfig},
    layers::{
        claims_id::claims_layer, session_id::SessionIdLayer, user_config_store::user_congig_store_layer,
        virtual_host_id::virtual_host_id_layer,
    },
    user_config_store::RedisUserConfigStore,
};

pub use crate::common::Config;

pub async fn run_gateway(config: Config) -> Result<()> {
    let redis_config = RedisConfig::try_from(&config)?;
    let redis_client = RedisClient::open(redis_config)?;

    // Create streamable HTTP service
    let mcp_service: StreamableHttpService<McpService, LocalSessionManager> = StreamableHttpService::new(
        move || Ok(McpService::new()),
        LocalSessionManager::default().into(),
        StreamableHttpServerConfig::default(),
    );

    let cors_layer = CorsLayer::new().allow_origin(Any).allow_methods(Any).allow_headers(Any).expose_headers(Any);

    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_audience(&[MCP_AUDIENCE]);

    let local_docoder = LocalDecoder::builder()
        .keys(vec![DecodingKey::from_rsa_pem(&fs::read(&config.token_verification_public_key)?)?])
        .validation(validation)
        .build()?;
    let mcp_add_state = McpGatewayAppState {
        jwt_token_decoder: Arc::new(local_docoder),
        config_store: Arc::new(RedisUserConfigStore::new(redis_client)),
        config: config.clone(),
    };

    let app = axum::Router::new()
        .nest_service("/servers/{virtual_host_name}/mcp", mcp_service)
        .layer(middleware::from_fn_with_state(mcp_add_state.clone(), user_congig_store_layer))
        .layer(middleware::from_fn_with_state(mcp_add_state.clone(), claims_layer))
        .layer(SessionIdLayer)
        .layer(middleware::from_fn(virtual_host_id_layer))
        .layer(cors_layer);

    #[cfg(feature = "with_tools")]
    let app = tools::add_tools(app);

    let app = app.with_state(mcp_add_state);

    let app = axum::Router::new().nest("/mcp-rs", app);

    // Start HTTP server

    let listener = tokio::net::TcpListener::bind(config.address).await?;
    tracing::info!("Server started on {}", config.address);

    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            tokio::signal::ctrl_c().await.ok();
            info!("Shutting down...");
        })
        .await?;

    Ok(())
}
