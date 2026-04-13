use std::{collections::HashMap, sync::Arc};

use rmcp::{RoleClient, model::InitializeRequestParams, service::RunningService};
use tokio::sync::Mutex;
use tracing::debug;

use crate::{
    gateway::{BackendTransportKey, BackendTransportService},
    layers::session_id::SessionId,
    user_config_store::VirtualHost,
};

pub struct SessionManager<'a> {
    virtual_host: &'a VirtualHost,
    session_id: &'a SessionId,
    transports: &'a Arc<Mutex<HashMap<BackendTransportKey, BackendTransportService>>>,
}

impl<'a> SessionManager<'a> {
    pub fn new(
        virtual_host: &'a VirtualHost,
        session_id: &'a SessionId,
        transports: &'a Arc<Mutex<HashMap<BackendTransportKey, BackendTransportService>>>,
    ) -> Self {
        Self { virtual_host, session_id, transports }
    }

    pub async fn borrow_transports(&self) -> Vec<(String, Option<RunningService<RoleClient, InitializeRequestParams>>)> {
        let names: Vec<_> = self.virtual_host.backends.keys().cloned().collect();
        let mut transports = self.transports.lock().await;
        names
            .into_iter()
            .filter_map(|name| transports.get_mut(&BackendTransportKey::from((&name, self.session_id))).map(|b| (name, b.service.take())))
            .collect()
    }

    pub async fn return_transports(
        &self,
        backend_transports: impl Iterator<Item = (String, Option<RunningService<RoleClient, InitializeRequestParams>>)>,
    ) {
        let mut transports = self.transports.lock().await;
        backend_transports.into_iter().for_each(|(name, svc)| {
            transports.entry(BackendTransportKey::from((&name, self.session_id))).and_modify(|e| e.service = svc);
        });
    }

    pub async fn cleanup_backends(&self, reason: &'static str) {
        let names: Vec<_> = self.virtual_host.backends.keys().cloned().collect();

        let mut transports = self.transports.lock().await;
        for name in names {
            let key = BackendTransportKey::from((&name, self.session_id));
            debug!("session_manager: removing transport for {key:?} {reason}");
            transports.remove(&key);
        }
    }
}
