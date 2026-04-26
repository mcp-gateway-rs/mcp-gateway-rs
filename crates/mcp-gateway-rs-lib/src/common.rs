use std::{path::PathBuf, sync::Arc};

use axum_jwt_auth::JwtDecoder;
use clap::Parser;
use http::uri::Authority;
use openid::{CompactJson, CustomClaims, StandardClaims};
use redis::{ConnectionAddr, IntoConnectionInfo};
use serde::{Deserialize, Serialize};

use std::net::SocketAddr;
use thiserror::Error;

use crate::user_config_store::UserConfigStore;

pub const MCP_AUDIENCE: &str = "mcp-audience";
pub const MCP_SESSION_ID: &str = "mcp-session-id";

#[derive(Clone)]
pub struct McpGatewayAppState {
    pub(crate) jwt_token_decoder: Arc<dyn JwtDecoder<McpGatewayClaims> + Send + Sync>,
    pub(crate) config_store: Arc<dyn UserConfigStore + Send + Sync>,
    pub(crate) config: Config,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct McpGatewayClaims {
    pub additional_claim: Option<String>,
    #[serde(flatten)]
    pub standard_claims: StandardClaims,
}

impl CustomClaims for McpGatewayClaims {
    fn standard_claims(&self) -> &StandardClaims {
        &self.standard_claims
    }
}

impl CompactJson for McpGatewayClaims {}

pub type RedisClient = redis::Client;

#[derive(Debug, Clone, Parser)]
pub struct RedisConfig {
    #[arg(long, env = "MCP_GATEWAY_RS_REDIS_HOSTNAME")]
    address: String,
    #[arg(long, env = "MCP_GATEWAY_RS_REDIS_PORT")]
    port: u16,
}

impl IntoConnectionInfo for RedisConfig {
    fn into_connection_info(self) -> redis::RedisResult<redis::ConnectionInfo> {
        ConnectionAddr::Tcp(self.address, self.port).into_connection_info()
    }
}

#[derive(Debug, Clone, Parser)]
#[command(name = "mcp-gateway-rs")]
#[command(about = "Minimal, fast and experimental MCP Gateway")]
pub struct Config {
    #[arg(long, env = "MCP_GATEWAY_RS_ADDRESS")]
    pub address: SocketAddr,
    #[arg(long, env = "MCP_GATEWAY_RS_REDIS_HOSTNAME")]
    pub redis_address: String,
    #[arg(long, env = "MCP_GATEWAY_RS_REDIS_PORT")]
    pub redis_port: u16,

    #[arg(long, env = "MCP_GATEWAY_TOKEN_VERIFICATION_PUBLIC_KEY")]
    pub token_verification_public_key: PathBuf,

    #[cfg(feature = "with_tools")]
    #[arg(long, env = "MCP_GATEWAY_TOKEN_VERIFICATION_PRIVATE_KEY")]
    pub token_verification_private_key: PathBuf,

    #[arg(long, env = "MCP_GATEWAY_ENABLE_OPEN_TELEMETRY")]
    pub enable_open_telemetry: Option<bool>,
}

#[derive(Error, Debug)]
pub enum ConfigValidationError {
    #[error("Redis Configuration Error")]
    RedisConfigurationError(String),
}

impl TryFrom<&Config> for RedisConfig {
    fn try_from(value: &Config) -> Result<Self, Self::Error> {
        let _: Authority = format!("{}:{}", value.redis_address, value.redis_port)
            .parse::<Authority>()
            .map_err(|e| ConfigValidationError::RedisConfigurationError(e.to_string()))?;
        Ok(Self { address: value.redis_address.clone(), port: value.redis_port })
    }

    type Error = ConfigValidationError;
}
