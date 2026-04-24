use std::sync::Arc;

use async_trait::async_trait;
use lru_cache::LruCache;
use redis::{AsyncCommands, RedisError, cmd};
use tokio::sync::Mutex;

use super::{ConfigStoreError, UserConfig, UserConfigStore};
use crate::{common::RedisClient, user_config_store::User};

#[derive(Debug, Clone)]
pub struct RedisUserConfigStore {
    redis_client: RedisClient,
    cache: Arc<Mutex<LruCache<String, UserConfig>>>,
}
impl RedisUserConfigStore {
    pub fn new(redis_client: RedisClient) -> Self {
        Self { redis_client, cache: Arc::new(Mutex::new(LruCache::new(50_000))) }
    }
}

#[async_trait]
impl UserConfigStore for RedisUserConfigStore {
    async fn get_config<'a>(&self, user_key: &'a User) -> Result<UserConfig, ConfigStoreError> {
        let has_key = { self.cache.lock().await.contains_key(user_key.key) };
        if !has_key {
            let Ok(key) = rmp_serde::encode::to_vec::<User>(user_key) else {
                return Err(ConfigStoreError::DataEncoding);
            };

            let Ok(mut connection) = self.redis_client.get_multiplexed_async_connection().await else {
                return Err(ConfigStoreError::InvalidConnection);
            };

            let maybe_user_config: Result<Option<Vec<u8>>, RedisError> =
                cmd("GET").arg(key).take().query_async(&mut connection).await;

            let Ok(Some(user_config)) = maybe_user_config else {
                return Err(ConfigStoreError::NoDataForKey);
            };

            let Ok(user_config) = rmp_serde::decode::from_slice::<UserConfig>(&user_config) else {
                return Err(ConfigStoreError::DataWrongFormat);
            };

            self.cache.lock().await.insert(user_key.key.to_owned(), user_config.clone());
            Ok(user_config)
        } else {
            if let Some(user_config) = self.cache.lock().await.get_mut(user_key.key) {
                Ok(user_config.clone())
            } else {
                return Err(ConfigStoreError::NoDataForKey);
            }
        }
    }

    async fn set_config<'a>(&self, user_key: &'a User, config: &'a UserConfig) -> Result<(), ConfigStoreError> {
        let Ok(key) = rmp_serde::encode::to_vec::<User>(user_key) else {
            return Err(ConfigStoreError::DataEncoding);
        };

        let Ok(encoded) = rmp_serde::encode::to_vec::<UserConfig>(config) else {
            return Err(ConfigStoreError::DataEncoding);
        };

        let Ok(mut connection) = self.redis_client.get_multiplexed_async_connection().await else {
            return Err(ConfigStoreError::InvalidConnection);
        };

        if connection.set::<&[u8], &[u8], String>(&key, &encoded).await.is_ok() {
            self.cache.lock().await.insert(user_key.key.to_owned(), config.clone());
            Ok(())
        } else {
            return Err(ConfigStoreError::CantWriteData);
        }
    }
}
