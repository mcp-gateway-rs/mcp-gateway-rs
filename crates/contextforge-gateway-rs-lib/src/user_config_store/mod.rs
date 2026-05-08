mod redis_config_store;
use async_trait::async_trait;
use contextforge_gateway_rs_apis::user_store::UserConfig;

pub use redis_config_store::RedisUserConfigStore;
use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone, Deserialize, Serialize, Hash, PartialEq, PartialOrd, Ord, Eq)]
pub struct User<'a> {
    name: &'static str,
    key: &'a str,
}

impl User<'_> {
    pub fn key(&self) -> &str {
        self.key
    }
}

impl<'a> User<'a> {
    pub fn new(key: &'a str) -> Self {
        Self { name: "UserConfig", key }
    }
}

#[async_trait]
pub trait UserConfigStore: Send + Sync {
    async fn get_config<'a>(&self, key: &'a User) -> Result<UserConfig, ConfigStoreError>;
    async fn set_config<'a>(&self, key: &'a User, user_config: &'a UserConfig) -> Result<(), ConfigStoreError>;
}
