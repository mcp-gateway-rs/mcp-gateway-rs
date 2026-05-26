//! Request-scoped RMCP notification forwarding from backend sessions.
//!
//! Backend notifications are captured through `ClientHandler` callbacks because
//! the gateway is an RMCP client of each backend. The active downstream
//! `tools/call` relay then forwards only the standard notifications that can be
//! scoped safely to that caller. This keeps streamable HTTP/SSE details inside
//! RMCP instead of proxying raw SSE frames.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};

use rmcp::{
    ClientHandler, RoleClient, RoleServer,
    model::{
        CallToolResult, InitializeRequestParams, LoggingLevel, LoggingMessageNotificationParam,
        ProgressNotificationParam, ProgressToken, ResourceUpdatedNotificationParam, ServerResult,
    },
    service::{NotificationContext, Peer, RequestHandle, RunningService, ServiceError},
};
use tokio::sync::{Mutex, broadcast};
use tracing::debug;

use super::mcp_call_validator::SessionKey;

const MAX_BUFFERED_NOTIFICATIONS_AFTER_RESPONSE: usize = 16;
const SESSION_RELAY_PEER_CLOSED_CHECK_INTERVAL: Duration = Duration::from_secs(30);

/// Typed backend notifications relayed at the RMCP boundary, not raw SSE frames.
#[derive(Clone, Debug)]
pub enum BackendNotification {
    Logging(LoggingMessageNotificationParam),
    Progress(ProgressNotificationParam),
    ResourceUpdated(ResourceUpdatedNotificationParam),
    ResourceListChanged,
    ToolListChanged,
}

/// Captures backend server-to-client notifications for the active request relay.
#[derive(Clone, Debug)]
pub struct BackendClientHandler {
    pub(crate) client_info: InitializeRequestParams,
    pub(crate) backend_notifications: broadcast::Sender<BackendNotification>,
}

impl ClientHandler for BackendClientHandler {
    fn get_info(&self) -> InitializeRequestParams {
        self.client_info.clone()
    }

    async fn on_logging_message(
        &self,
        params: LoggingMessageNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) {
        self.send_notification(BackendNotification::Logging(params));
    }

    async fn on_progress(&self, params: ProgressNotificationParam, _context: NotificationContext<RoleClient>) {
        self.send_notification(BackendNotification::Progress(params));
    }

    async fn on_resource_updated(
        &self,
        params: ResourceUpdatedNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) {
        self.send_notification(BackendNotification::ResourceUpdated(params));
    }

    async fn on_resource_list_changed(&self, _context: NotificationContext<RoleClient>) {
        self.send_notification(BackendNotification::ResourceListChanged);
    }

    async fn on_tool_list_changed(&self, _context: NotificationContext<RoleClient>) {
        self.send_notification(BackendNotification::ToolListChanged);
    }
}

impl BackendClientHandler {
    fn send_notification(&self, notification: BackendNotification) {
        let _ = self.backend_notifications.send(notification);
    }
}

pub type BackendClientService = Arc<RunningService<RoleClient, BackendClientHandler>>;
pub(crate) type ResourceSubscriptions = Arc<Mutex<HashMap<SessionKey, HashSet<String>>>>;
pub(crate) type LogLevels = Arc<Mutex<HashMap<SessionKey, LoggingLevel>>>;

struct RelayScope<'a> {
    backend_name: &'a str,
    resource_subscriptions: &'a ResourceSubscriptions,
    log_levels: &'a LogLevels,
    session_key: &'a SessionKey,
}

struct RequestScope<'a> {
    downstream_progress_token: Option<&'a ProgressToken>,
    upstream_progress_token: &'a ProgressToken,
}

pub(crate) struct SessionNotificationRelay {
    pub notification_rx: broadcast::Receiver<BackendNotification>,
    pub backend_name: String,
    pub downstream_peer: Peer<RoleServer>,
    pub resource_subscriptions: ResourceSubscriptions,
    pub log_levels: LogLevels,
    pub session_key: SessionKey,
}

/// Relays backend notifications only while the routed tool call is active.
pub(crate) async fn await_call_tool_response_with_notifications(
    mut notification_rx: Option<broadcast::Receiver<BackendNotification>>,
    downstream_peer: Peer<RoleServer>,
    downstream_progress_token: Option<ProgressToken>,
    request_handle: RequestHandle<RoleClient>,
) -> Result<CallToolResult, ServiceError> {
    let upstream_progress_token = request_handle.progress_token.clone();
    let response = request_handle.await_response();
    tokio::pin!(response);

    let Some(notification_rx) = notification_rx.as_mut() else {
        return call_tool_result(response.await);
    };

    let scope = RequestScope {
        downstream_progress_token: downstream_progress_token.as_ref(),
        upstream_progress_token: &upstream_progress_token,
    };

    loop {
        tokio::select! {
            response = &mut response => {
                drain_buffered_notifications(notification_rx, &downstream_peer, &scope).await;
                return call_tool_result(response);
            },
            event = notification_rx.recv() => match event {
                Ok(notification) => {
                    forward_request_notification(&downstream_peer, notification, &scope).await;
                },
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    debug!(skipped, "backend notification receiver lagged");
                },
                Err(broadcast::error::RecvError::Closed) => break,
            },
        }
    }

    call_tool_result(response.await)
}

pub(crate) async fn relay_backend_notifications(mut relay: SessionNotificationRelay) {
    let scope = RelayScope {
        backend_name: &relay.backend_name,
        resource_subscriptions: &relay.resource_subscriptions,
        log_levels: &relay.log_levels,
        session_key: &relay.session_key,
    };

    loop {
        if relay.downstream_peer.is_transport_closed() {
            break;
        }
        tokio::select! {
            event = relay.notification_rx.recv() => match event {
                Ok(notification) => {
                    if !forward_session_notification(&relay.downstream_peer, notification, &scope).await {
                        break;
                    }
                },
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    debug!(skipped, "backend session notification receiver lagged");
                },
                Err(broadcast::error::RecvError::Closed) => break,
            },
            () = tokio::time::sleep(SESSION_RELAY_PEER_CLOSED_CHECK_INTERVAL) => {},
        }
    }
}

pub(crate) async fn drain_buffered_session_notifications(
    mut relay: SessionNotificationRelay,
) -> Option<broadcast::Receiver<BackendNotification>> {
    let scope = RelayScope {
        backend_name: &relay.backend_name,
        resource_subscriptions: &relay.resource_subscriptions,
        log_levels: &relay.log_levels,
        session_key: &relay.session_key,
    };

    let mut drained = 0;
    while let Some(notification) = next_buffered_notification(&mut relay.notification_rx, &mut drained) {
        if !forward_session_notification(&relay.downstream_peer, notification, &scope).await {
            return None;
        }
    }
    Some(relay.notification_rx)
}

async fn drain_buffered_notifications(
    notification_rx: &mut broadcast::Receiver<BackendNotification>,
    downstream_peer: &Peer<RoleServer>,
    scope: &RequestScope<'_>,
) {
    let mut drained = 0;
    while let Some(notification) = next_buffered_notification(notification_rx, &mut drained) {
        forward_request_notification(downstream_peer, notification, scope).await;
    }
}

fn next_buffered_notification(
    notification_rx: &mut broadcast::Receiver<BackendNotification>,
    drained: &mut usize,
) -> Option<BackendNotification> {
    if *drained >= MAX_BUFFERED_NOTIFICATIONS_AFTER_RESPONSE {
        debug!(drained, "stopped draining backend notifications after response");
        return None;
    }

    loop {
        match notification_rx.try_recv() {
            Ok(notification) => {
                *drained += 1;
                return Some(notification);
            },
            Err(broadcast::error::TryRecvError::Lagged(skipped)) => {
                debug!(skipped, "backend notification receiver lagged while draining");
            },
            Err(broadcast::error::TryRecvError::Empty | broadcast::error::TryRecvError::Closed) => return None,
        }
    }
}

fn call_tool_result(response: Result<ServerResult, ServiceError>) -> Result<CallToolResult, ServiceError> {
    match response? {
        ServerResult::CallToolResult(result) => Ok(result),
        _ => Err(ServiceError::UnexpectedResponse),
    }
}

async fn forward_request_notification(
    downstream_peer: &Peer<RoleServer>,
    notification: BackendNotification,
    scope: &RequestScope<'_>,
) {
    let Some(notification) = scope_request_notification(notification, scope) else {
        return;
    };

    if let BackendNotification::Progress(params) = notification {
        let _ = downstream_peer.notify_progress(params).await;
    }
}

async fn forward_session_notification(
    downstream_peer: &Peer<RoleServer>,
    notification: BackendNotification,
    scope: &RelayScope<'_>,
) -> bool {
    let Some(notification) = scope_session_notification(notification, scope).await else {
        return true;
    };

    match notification {
        BackendNotification::Logging(params) => downstream_peer.notify_logging_message(params).await.is_ok(),
        BackendNotification::ResourceUpdated(params) => downstream_peer.notify_resource_updated(params).await.is_ok(),
        BackendNotification::ResourceListChanged => downstream_peer.notify_resource_list_changed().await.is_ok(),
        BackendNotification::ToolListChanged => downstream_peer.notify_tool_list_changed().await.is_ok(),
        BackendNotification::Progress(_) => true,
    }
}

fn scope_request_notification(
    notification: BackendNotification,
    scope: &RequestScope<'_>,
) -> Option<BackendNotification> {
    match notification {
        BackendNotification::Progress(mut params) => {
            if &params.progress_token != scope.upstream_progress_token {
                return None;
            }
            params.progress_token = scope.downstream_progress_token?.clone();
            Some(BackendNotification::Progress(params))
        },
        _ => None,
    }
}

async fn scope_session_notification(
    notification: BackendNotification,
    scope: &RelayScope<'_>,
) -> Option<BackendNotification> {
    match notification {
        BackendNotification::Logging(params) => {
            let log_level =
                scope.log_levels.lock().await.get(scope.session_key).copied().unwrap_or(LoggingLevel::Debug);
            if !logging_level_enabled(params.level, log_level) {
                return None;
            }
            Some(BackendNotification::Logging(params))
        },
        BackendNotification::ResourceUpdated(mut params) => {
            let subscriptions = scope.resource_subscriptions.lock().await;
            let session_subscriptions = subscriptions.get(scope.session_key)?;
            let scoped_uri = format!("{}-{}", scope.backend_name, params.uri);
            let is_subscribed = session_subscriptions.contains(&scoped_uri)
                || session_subscriptions.iter().any(|subscribed| resource_uri_matches(subscribed, &scoped_uri));
            if !is_subscribed {
                return None;
            }
            params.uri = scoped_uri;
            Some(BackendNotification::ResourceUpdated(params))
        },
        BackendNotification::ResourceListChanged => Some(BackendNotification::ResourceListChanged),
        BackendNotification::ToolListChanged => Some(BackendNotification::ToolListChanged),
        BackendNotification::Progress(_) => None,
    }
}

fn resource_uri_matches(subscribed: &str, updated: &str) -> bool {
    updated == subscribed || updated.strip_prefix(subscribed).is_some_and(|suffix| suffix.starts_with('/'))
}

fn logging_level_enabled(level: LoggingLevel, minimum: LoggingLevel) -> bool {
    logging_level_rank(level) >= logging_level_rank(minimum)
}

fn logging_level_rank(level: LoggingLevel) -> u8 {
    // RMCP's `LoggingLevel` does not implement `Ord`; keep the MCP severity order local.
    match level {
        LoggingLevel::Debug => 0,
        LoggingLevel::Info => 1,
        LoggingLevel::Notice => 2,
        LoggingLevel::Warning => 3,
        LoggingLevel::Error => 4,
        LoggingLevel::Critical => 5,
        LoggingLevel::Alert => 6,
        LoggingLevel::Emergency => 7,
    }
}

#[cfg(test)]
mod tests {
    use rmcp::model::NumberOrString;

    use super::*;

    fn progress_notification(progress_token: ProgressToken) -> BackendNotification {
        BackendNotification::Progress(ProgressNotificationParam {
            progress_token,
            progress: 1.0,
            total: Some(1.0),
            message: Some("progress".to_owned()),
        })
    }

    fn session_key() -> SessionKey {
        SessionKey::new("subject", "virtual-host", "session")
    }

    fn resource_subscriptions(session_key: &SessionKey, uris: &[&str]) -> ResourceSubscriptions {
        Arc::new(Mutex::new(HashMap::from([(session_key.clone(), uris.iter().map(|uri| (*uri).to_owned()).collect())])))
    }

    fn scope_progress_for_test(
        notification: BackendNotification,
        downstream_progress_token: Option<&ProgressToken>,
        upstream_progress_token: &ProgressToken,
    ) -> Option<BackendNotification> {
        scope_request_notification(notification, &RequestScope { downstream_progress_token, upstream_progress_token })
    }

    async fn scope_session_for_test(
        notification: BackendNotification,
        resource_subscriptions: &ResourceSubscriptions,
    ) -> Option<BackendNotification> {
        let log_levels = Arc::new(Mutex::new(HashMap::new()));
        let session_key = session_key();
        scope_session_notification(
            notification,
            &RelayScope {
                backend_name: "backend",
                resource_subscriptions,
                log_levels: &log_levels,
                session_key: &session_key,
            },
        )
        .await
    }

    #[tokio::test]
    async fn scopes_progress_to_downstream_token_only() {
        let upstream_token = ProgressToken(NumberOrString::String("upstream".into()));
        let downstream_token = ProgressToken(NumberOrString::String("downstream".into()));
        let scoped = scope_progress_for_test(
            progress_notification(upstream_token.clone()),
            Some(&downstream_token),
            &upstream_token,
        );

        match scoped {
            Some(BackendNotification::Progress(params)) => assert_eq!(params.progress_token, downstream_token),
            other => panic!("expected scoped progress notification, got {other:?}"),
        }

        let no_downstream_token =
            scope_progress_for_test(progress_notification(upstream_token.clone()), None, &upstream_token);
        assert!(no_downstream_token.is_none());

        let stale = scope_progress_for_test(
            progress_notification(ProgressToken(NumberOrString::String("wrong".into()))),
            Some(&ProgressToken(NumberOrString::String("downstream".into()))),
            &upstream_token,
        );
        assert!(stale.is_none());
    }

    #[tokio::test]
    async fn scopes_resource_updated_to_subscribed_uri_or_sub_resource() {
        let session_key = session_key();
        let subscribed_resources = resource_subscriptions(&session_key, &["backend-memo://insights"]);
        let scoped = scope_session_for_test(
            BackendNotification::ResourceUpdated(ResourceUpdatedNotificationParam { uri: "memo://insights".into() }),
            &subscribed_resources,
        )
        .await;

        match scoped {
            Some(BackendNotification::ResourceUpdated(params)) => assert_eq!(params.uri, "backend-memo://insights"),
            other => panic!("expected subscribed resource update, got {other:?}"),
        }

        let sub_resource = scope_session_for_test(
            BackendNotification::ResourceUpdated(ResourceUpdatedNotificationParam {
                uri: "memo://insights/item/1".into(),
            }),
            &subscribed_resources,
        )
        .await;
        match sub_resource {
            Some(BackendNotification::ResourceUpdated(params)) => {
                assert_eq!(params.uri, "backend-memo://insights/item/1");
            },
            other => panic!("expected subscribed sub-resource update, got {other:?}"),
        }

        let no_subscriptions = resource_subscriptions(&session_key, &[]);
        let unsubscribed = scope_session_for_test(
            BackendNotification::ResourceUpdated(ResourceUpdatedNotificationParam { uri: "memo://insights".into() }),
            &no_subscriptions,
        )
        .await;
        assert!(unsubscribed.is_none());
    }

    #[tokio::test]
    async fn scopes_resource_updated_by_session_key() {
        let other_session_key = SessionKey::new("other-subject", "virtual-host", "session");
        let subscriptions = resource_subscriptions(&other_session_key, &["backend-memo://insights"]);
        let scoped = scope_session_for_test(
            BackendNotification::ResourceUpdated(ResourceUpdatedNotificationParam { uri: "memo://insights".into() }),
            &subscriptions,
        )
        .await;

        assert!(scoped.is_none());
    }

    #[test]
    fn drain_collects_only_configured_buffer() {
        let (tx, mut rx) = broadcast::channel(MAX_BUFFERED_NOTIFICATIONS_AFTER_RESPONSE * 2);
        for _ in 0..=MAX_BUFFERED_NOTIFICATIONS_AFTER_RESPONSE {
            tx.send(BackendNotification::Logging(LoggingMessageNotificationParam {
                level: LoggingLevel::Info,
                logger: None,
                data: serde_json::Value::Null,
            }))
            .expect("receiver should accept test notification");
        }

        let mut drained = 0;
        let mut buffered = Vec::new();
        while let Some(notification) = next_buffered_notification(&mut rx, &mut drained) {
            buffered.push(notification);
        }

        assert_eq!(buffered.len(), MAX_BUFFERED_NOTIFICATIONS_AFTER_RESPONSE);
        assert!(matches!(rx.try_recv(), Ok(BackendNotification::Logging(_))));
    }

    #[test]
    fn logging_level_filter_uses_mcp_severity_order() {
        assert!(logging_level_enabled(LoggingLevel::Emergency, LoggingLevel::Error));
        assert!(logging_level_enabled(LoggingLevel::Error, LoggingLevel::Error));
        assert!(!logging_level_enabled(LoggingLevel::Warning, LoggingLevel::Error));
    }
}
