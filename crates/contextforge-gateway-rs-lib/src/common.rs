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

#[derive(Clone)]
pub struct ContextForgeGatewayAppState {
    pub(crate) jwt_token_decoder: Arc<dyn JwtDecoder<ContextForgeGatewayClaims> + Send + Sync>,
    pub(crate) config_store: Arc<dyn UserConfigStore + Send + Sync>,
    pub(crate) config: Config,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ContextForgeGatewayClaims {
    pub additional_claim: Option<String>,
    #[serde(flatten)]
    pub standard_claims: StandardClaims,
}

impl CustomClaims for ContextForgeGatewayClaims {
    fn standard_claims(&self) -> &StandardClaims {
        &self.standard_claims
    }
}

impl CompactJson for ContextForgeGatewayClaims {}

pub type RedisClient = redis::Client;

#[derive(Debug, Clone)]
pub struct RedisConfig {
    address: String,
    port: u16,
}

impl IntoConnectionInfo for RedisConfig {
    fn into_connection_info(self) -> redis::RedisResult<redis::ConnectionInfo> {
        ConnectionAddr::Tcp(self.address, self.port).into_connection_info()
    }
}

#[derive(Debug, Clone, Parser)]
#[command(name = "contextforge-gateway-rs")]
#[command(about = "Minimal, fast and experimental Gateway/Dataplane for ContextForge")]
pub struct Config {
    #[arg(long, env = "CONTEXTFORGE_GATEWAY_RS_ADDRESS")]
    pub address: Option<SocketAddr>,
    #[arg(long, env = "CONTEXTFORGE_GATEWAY_RS_REDIS_HOSTNAME")]
    pub redis_address: String,
    #[arg(long, env = "CONTEXTFORGE_GATEWAY_RS_REDIS_PORT")]
    pub redis_port: u16,

    #[arg(long, env = "CONTEXTFORGE_GATEWAY_RS_TOKEN_VERIFICATION_PUBLIC_KEY")]
    pub token_verification_public_key: PathBuf,

    #[cfg(feature = "with_tools")]
    #[arg(long, env = "CONTEXTFORGE_GATEWAY_RS_TOKEN_VERIFICATION_PRIVATE_KEY")]
    pub token_verification_private_key: PathBuf,

    #[arg(long, env = "CONTEXTFORGE_GATEWAY_RS_ENABLE_OPEN_TELEMETRY")]
    pub enable_open_telemetry: Option<bool>,

    #[arg(long, env = "CONTEXTFORGE_GATEWAY_RS_GATEWAY_CPUS")]
    pub number_of_cpus: Option<usize>,

    #[arg(long, env = "CONTEXTFORGE_GATEWAY_RS_SINGLE_RUNTIME")]
    pub single_runtime: Option<bool>,

    #[arg(long, env = "CONTEXTFORGE_GATEWAY_RS_TLS_ADDRESS")]
    pub tls_address: Option<SocketAddr>,

    #[arg(long, env = "CONTEXTFORGE_GATEWAY_RS_TLS_SERVER_PRIVATE_KEY")]
    pub server_private_key: Option<PathBuf>,

    #[arg(long, env = "CONTEXTFORGE_GATEWAY_RS_TLS_SERVER_CERTIFICATE")]
    pub server_certificate: Option<PathBuf>,
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
