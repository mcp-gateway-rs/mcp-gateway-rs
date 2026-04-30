use std::{fs, sync::Arc};

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
mod tcp;
#[cfg(feature = "with_tools")]
mod tools;
mod user_config_store;
use gateway::McpService;
use layers::session_id::SessionId;

use tcp::Tcp;
use tokio::net::TcpListener;
use tower_http::cors::{Any, CorsLayer};
use tracing::info;

use crate::{
    common::{MCP_AUDIENCE, McpGatewayAppState, RedisClient, RedisConfig},
    gateway::{LocalUserSessionStore, RedisUserSessionStore},
    layers::{
        claims_id::claims_layer, session_id::SessionIdLayer, user_config_store::user_config_store_layer,
        virtual_host_id::virtual_host_id_layer,
    },
    user_config_store::RedisUserConfigStore,
};

pub use crate::common::Config;

pub async fn run_gateway(
    config: Config,
    local_session_manager: Arc<LocalSessionManager>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let redis_config = RedisConfig::try_from(&config)?;
    let redis_client = RedisClient::open(redis_config)?;
    let user_session_store = LocalUserSessionStore::new();
    //let user_session_store = RedisUserSessionStore::new(redis_client.clone());
    //let redis_session_store = RedisSessionStore::new(redis_client.clone());

    let streamable_config = StreamableHttpServerConfig::default().disable_allowed_hosts();

    // Create streamable HTTP service
    let mcp_service: StreamableHttpService<McpService<LocalUserSessionStore>, LocalSessionManager> =
        StreamableHttpService::new(
            move || Ok(McpService::with_stores(user_session_store.clone())),
            //Arc::new(LORemoteSessionManager::new(redis_session_store)),
            local_session_manager,
            streamable_config,
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
        .layer(middleware::from_fn_with_state(mcp_add_state.clone(), user_config_store_layer))
        .layer(middleware::from_fn_with_state(mcp_add_state.clone(), claims_layer))
        .layer(SessionIdLayer)
        .layer(middleware::from_fn(virtual_host_id_layer))
        .layer(cors_layer);

    #[cfg(feature = "with_tools")]
    let app = tools::add_tools(app);

    let app = app.with_state(mcp_add_state);

    let app = axum::Router::new().nest("/mcp-rs", app);

    let listener = Tcp::new(config.address);

    tracing::info!("Server started on {}", config.address);
    let tcp_listener: TcpListener = listener.try_into()?;
    axum::serve(tcp_listener, app)
        .with_graceful_shutdown(async {
            tokio::signal::ctrl_c().await.ok();
            info!("Shutting down...");
        })
        .await?;

    Ok(())
}
