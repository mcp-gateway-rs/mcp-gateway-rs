use async_trait::async_trait;
use redis::{AsyncCommands, RedisError, cmd};

use super::{ConfigStoreError, UserConfig, UserConfigStore};
use crate::common::RedisClient;

#[derive(Debug, Clone)]
pub struct RedisUserConfigStore {
    redis_client: RedisClient,
}
impl RedisUserConfigStore {
    pub fn new(redis_client: RedisClient) -> Self {
        Self { redis_client }
    }
}

#[async_trait]
impl UserConfigStore for RedisUserConfigStore {
    async fn get_config<'a>(&self, key: &'a str) -> Result<UserConfig, ConfigStoreError> {
        let Ok(mut connection) = self.redis_client.get_multiplexed_async_connection().await else {
            return Err(ConfigStoreError::InvalidConnection);
        };

        let maybe_user_config: Result<Option<Vec<u8>>, RedisError> = cmd("GET")
            .arg(key)
            .take()
            .query_async(&mut connection)
            .await;

        let Ok(Some(user_config)) = maybe_user_config else {
            return Err(ConfigStoreError::NoDataForKey);
        };

        let Ok(user_config) = rmp_serde::decode::from_slice::<UserConfig>(&user_config) else {
            return Err(ConfigStoreError::DataWrongFormat);
        };

        Ok(user_config)
    }

    async fn set_config<'a>(
        &self,
        key: &'a str,
        config: &'a UserConfig,
    ) -> Result<(), ConfigStoreError> {
        let Ok(encoded) = rmp_serde::encode::to_vec::<UserConfig>(config) else {
            return Err(ConfigStoreError::DataEncoding);
        };

        let Ok(mut connection) = self.redis_client.get_multiplexed_async_connection().await else {
            return Err(ConfigStoreError::InvalidConnection);
        };

        if connection
            .set::<&str, &[u8], String>(key, &encoded)
            .await
            .is_ok()
        {
            Ok(())
        } else {
            return Err(ConfigStoreError::CantWriteData);
        }
    }
}
