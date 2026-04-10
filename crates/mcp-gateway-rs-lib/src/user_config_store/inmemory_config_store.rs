use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use tokio::sync::Mutex;

use super::{ConfigStoreError, UserConfig, UserConfigStore};

#[derive(Debug, Clone)]
pub struct InMemoryUserConfigStore {
    store: Arc<Mutex<HashMap<String, Vec<u8>>>>,
}
impl InMemoryUserConfigStore {
    pub fn new() -> Self {
        Self {
            store: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

#[async_trait]
impl UserConfigStore for InMemoryUserConfigStore {
    async fn get_config<'a>(&self, key: &'a str) -> Result<UserConfig, ConfigStoreError> {
        let store = self.store.lock().await;
        let Some(user_config) = store.get(key) else {
            return Err(ConfigStoreError::NoDataForKey);
        };

        let Ok(user_config) = rmp_serde::decode::from_slice::<UserConfig>(user_config) else {
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
        let mut store = self.store.lock().await;
        let _ = store.insert(key.to_owned(), encoded);
        Ok(())
    }
}
