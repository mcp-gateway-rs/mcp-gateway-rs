use std::time::Duration;

pub const LRU_CACHE_ENTRIES: usize = 50_000;
pub const LRU_CACHE_EXPIRY_DURATION: Duration = Duration::from_hours(1);
pub const CONTEXT_FORGE_GATEWAY_AUDIENCE: &str = "mcpgateway-api";
pub const CONTEXT_FORGE_GATEWAY_ISSUER: &str = "mcpgateway";
pub const MCP_SESSION_ID: &str = "mcp-session-id";
pub const REDIS_RETRIES: usize = 1000; // keep re-trying forver
