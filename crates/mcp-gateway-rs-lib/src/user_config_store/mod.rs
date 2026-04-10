//mod inmemory_config_store;
mod redis_config_store;

use std::collections::HashMap;

use async_trait::async_trait;

use serde::{Deserialize, Serialize};

//pub use inmemory_config_store::InMemoryUserConfigStore;
pub use redis_config_store::RedisUserConfigStore;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendMCPGateway {
    pub url: url::Url,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VirtualHost {
    pub backends: HashMap<String, BackendMCPGateway>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserConfig {
    pub virtual_hosts: HashMap<String, VirtualHost>,
}

#[derive(Debug, Clone, Deserialize, Serialize, thiserror::Error)]
pub enum ConfigStoreError {
    #[error("data store disconnected")]
    InvalidConnection,
    #[error("no data for key")]
    NoDataForKey,
    #[error("data in wrong format")]
    DataWrongFormat,
    #[error("unable to encode the data")]
    DataEncoding,
    #[error("unable to write to store")]
    CantWriteData,
}

#[async_trait]
pub trait UserConfigStore: Send + Sync {
    async fn get_config<'a>(&self, key: &'a str) -> Result<UserConfig, ConfigStoreError>;
    async fn set_config<'a>(
        &self,
        key: &'a str,
        user_config: &'a UserConfig,
    ) -> Result<(), ConfigStoreError>;
}
