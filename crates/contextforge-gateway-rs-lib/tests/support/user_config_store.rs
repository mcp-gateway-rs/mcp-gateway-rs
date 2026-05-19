use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use contextforge_gateway_rs_apis::{User, user_store::UserConfig};
use contextforge_gateway_rs_lib::{ConfigStoreError, UserConfigStore};
use tokio::sync::Mutex;

#[derive(Clone, Default)]
pub(crate) struct MemoryUserConfigStore {
    configs: Arc<Mutex<HashMap<String, UserConfig>>>,
}

#[async_trait]
impl UserConfigStore for MemoryUserConfigStore {
    async fn get_config<'a>(&self, key: &'a User<'a>) -> Result<UserConfig, ConfigStoreError> {
        self.configs.lock().await.get(key.key()).cloned().ok_or(ConfigStoreError::NoDataForKey)
    }

    async fn set_config<'a>(&self, key: &'a User<'a>, config: &'a UserConfig) -> Result<(), ConfigStoreError> {
        self.configs.lock().await.insert(key.key().to_owned(), config.clone());
        Ok(())
    }
}
