mod mcp_call_validator;
mod session_manager;
use std::collections::HashMap;
use std::{collections::HashSet, sync::Arc};

use http::request::Parts;
use itertools::Itertools;
use mcp_call_validator::AuthorizedCallValidator;
use rmcp::RoleClient;
use rmcp::model::ErrorCode;
use rmcp::service::RunningService;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::{NewSessionId, StreamableHttpClientTransport};
use rmcp::{
    ErrorData, RoleServer, ServerHandler, ServiceExt,
    model::{
        AnnotateAble, CallToolRequestParams, CallToolResult, CompleteRequestParams, CompleteResult, CompletionInfo, GetPromptRequestParams,
        GetPromptResult, Implementation, InitializeRequestParams, InitializeResult, ListPromptsResult, ListResourceTemplatesResult,
        ListResourcesResult, ListToolsResult, LoggingLevel, PaginatedRequestParams, Prompt, PromptArgument, PromptMessage,
        PromptMessageContent, PromptMessageRole, RawImageContent, RawResource, RawResourceTemplate, ReadResourceRequestParams,
        ReadResourceResult, Reference, ResourceContents, ServerCapabilities, SetLevelRequestParams, SubscribeRequestParams, Tool,
        UnsubscribeRequestParams,
    },
    service::RequestContext,
};

use tokio::sync::Mutex;
use tracing::{debug, info, warn};

use crate::gateway::session_manager::SessionManager;
use crate::layers::virtual_host_id::VirtualHostId;
use crate::{SessionId, user_config_store::UserConfig};

#[derive(Clone)]
pub struct McpService {
    subscriptions: Arc<Mutex<HashSet<String>>>,
    transports: Arc<Mutex<HashMap<BackendTransportKey, BackendTransportService>>>,
    log_level: Arc<Mutex<LoggingLevel>>,
    http_client: reqwest::Client,
}

impl McpService {
    pub fn new() -> Self {
        Self {
            subscriptions: Arc::new(Mutex::new(HashSet::new())),
            transports: Arc::new(Mutex::new(HashMap::new())),
            log_level: Arc::new(Mutex::new(LoggingLevel::Debug)),
            http_client: reqwest::Client::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct BackendTransportKey {
    backend_name: String,
    session_id: String,
}

type BackendService = RunningService<RoleClient, InitializeRequestParams>;

#[derive(Debug)]
struct BackendTransportService {
    capabilities: Option<ServerCapabilities>,
    service: Option<BackendService>,
}

impl From<(&str, &str)> for BackendTransportKey {
    fn from((backend_name, session_name): (&str, &str)) -> Self {
        Self { backend_name: backend_name.to_owned(), session_id: session_name.to_owned() }
    }
}

impl From<(&String, &SessionId)> for BackendTransportKey {
    fn from((backend_name, session_name): (&String, &SessionId)) -> Self {
        Self { backend_name: backend_name.to_owned(), session_id: session_name.value().to_owned() }
    }
}

impl From<(Option<ServerCapabilities>, Option<BackendService>)> for BackendTransportService {
    fn from((capabilities, service): (Option<ServerCapabilities>, Option<BackendService>)) -> Self {
        Self { capabilities, service }
    }
}

impl ServerHandler for McpService {
    async fn initialize(&self, request: InitializeRequestParams, cx: RequestContext<RoleServer>) -> Result<InitializeResult, ErrorData> {
        let maybe_parts = cx.extensions.get::<Parts>();

        let maybe_new_session = cx.extensions.get::<NewSessionId>();
        let maybe_user_config = maybe_parts.and_then(|parts| parts.extensions.get::<UserConfig>());
        let maybe_virtual_host_id = maybe_parts.and_then(|parts| parts.extensions.get::<VirtualHostId>());
        info!(
            "intialize user_config = {maybe_user_config:#?} new_session_id = {maybe_new_session:#?} virtual_host_id = {maybe_virtual_host_id:#?}"
        );

        let Some(new_session) = maybe_new_session else {
            return Err(ErrorData {
                code: ErrorCode::INTERNAL_ERROR,
                message: "Routing problem... session id not created".into(),
                data: None,
            });
        };

        let Some(user_config) = maybe_user_config else {
            return Err(ErrorData {
                code: ErrorCode::INTERNAL_ERROR,
                message: "Routing problem... user config not found".into(),
                data: None,
            });
        };

        let Some(virtual_host_id) = maybe_virtual_host_id else {
            return Err(ErrorData {
                code: ErrorCode::INTERNAL_ERROR,
                message: "Routing problem... virutal host not known".into(),
                data: None,
            });
        };

        let Some(virtual_host) = user_config.virtual_hosts.get(virtual_host_id.value()) else {
            return Err(ErrorData { code: ErrorCode::RESOURCE_NOT_FOUND, message: "No configuration".into(), data: None });
        };

        let tasks = virtual_host
            .backends
            .iter()
            .map(|(name, backend)| {
                let client = self.http_client.clone();
                let request = request.clone();
                let backend_url = backend.url.clone();
                let new_session_id = new_session.clone();

                Box::pin(async move {
                    let config = StreamableHttpClientTransportConfig::with_uri(backend_url.to_string());
                    let transport = StreamableHttpClientTransport::with_client(client, config);
                    let maybe_running_service = request.serve(transport).await;
                    if let Ok(running_service) = maybe_running_service {
                        (name, Some(running_service))
                    } else {
                        warn!("initialize: Unable to initialize for {new_session_id:?} {name:?} {maybe_running_service:?}",);
                        (name, None)
                    }
                })
            })
            .collect::<Vec<_>>();
        let initialization_results = futures::future::join_all(tasks).await;

        let (capabilities, backend_services): (Vec<_>, Vec<_>) = initialization_results
            .into_iter()
            .map(|(name, running_service)| {
                info!("initialize: Adding transport: session_id {new_session:#?} backend {name} {running_service:?}");
                let server_capabilities =
                    running_service.as_ref().and_then(|rs| rs.peer().peer_info().as_ref().map(|pi| pi.capabilities.clone()));
                (
                    (name.clone(), server_capabilities.clone()),
                    (name.clone(), BackendTransportService::from((server_capabilities, running_service))),
                )
            })
            .unzip();

        let mut transports = self.transports.lock().await;
        for (name, svc) in backend_services {
            transports.entry(BackendTransportKey::from((name.as_str(), new_session.value()))).insert_entry(svc);
        }
        drop(transports);

        Ok(InitializeResult::new(merge_capabilities(capabilities))
            .with_server_info(Implementation::new("rust-conformance-server", "0.1.0"))
            .with_instructions("Rust MCP conformance test server"))
    }

    async fn ping(&self, _cx: RequestContext<RoleServer>) -> Result<(), ErrorData> {
        Ok(())
    }

    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        cx: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let mcp_call_validator = AuthorizedCallValidator::new("list_tools", &cx);
        let (virtual_host, session_id) = mcp_call_validator.validate()?;

        let session_manager = SessionManager::new(virtual_host, session_id, &self.transports);
        let backend_transports = session_manager.borrow_transports().await;

        let list_tools_tasks = backend_transports
            .into_iter()
            .map(|(name, backend)| {
                let request = request.clone();
                async move {
                    if let Some(service) = backend {
                        let response = service.list_tools(request).await;
                        (name, Some(service), Some(response))
                    } else {
                        (name, None, None)
                    }
                }
            })
            .collect::<Vec<_>>();

        let list_tools_tasks_results: Vec<(String, Option<_>, Option<_>)> = futures::future::join_all(list_tools_tasks).await;

        let (backend_services, responses): (Vec<_>, Vec<_>) = list_tools_tasks_results
            .into_iter()
            .map(|(name, service, response)| {
                info!("list_tools: backend {name} {response:?}");
                ((name.clone(), service), (name, response))
            })
            .unzip();

        let mut transports = self.transports.lock().await;
        for (name, svc) in backend_services {
            transports.entry(BackendTransportKey::from((&name, session_id))).and_modify(|e| e.service = svc);
        }
        drop(transports);

        let responses = responses
            .into_iter()
            .filter_map(|(name, response)| if let Some(Ok(response)) = response { Some((name, response)) } else { None })
            .collect::<Vec<_>>();

        let merged_list_tools = merge_tools(responses);

        Ok(ListToolsResult { meta: None, tools: merged_list_tools, next_cursor: None })
    }

    async fn call_tool(&self, request: CallToolRequestParams, cx: RequestContext<RoleServer>) -> Result<CallToolResult, ErrorData> {
        let mcp_call_validator = AuthorizedCallValidator::new("call_tool", &cx);
        let (virtual_host, session_id) = mcp_call_validator.validate()?;
        let session_manager = SessionManager::new(virtual_host, session_id, &self.transports);

        let Some((backend_name, tool_name)) = split_tool_name(&request.name) else {
            return Err(ErrorData {
                code: ErrorCode::INTERNAL_ERROR,
                message: "Routing problem... session id not created".into(),
                data: None,
            });
        };

        let backend_transports = session_manager.borrow_transports().await;

        let (services, call_tool_tasks): (Vec<_>, Vec<_>) = backend_transports
            .into_iter()
            .map(|(name, backend_transport)| {
                debug!(
                    "call_tool: Finding backend for {} {name} {backend_name} tool_name = {tool_name} backend={}",
                    &request.name,
                    backend_transport.is_some(),
                );
                if name == backend_name {
                    let mut request = request.clone();
                    request.name = tool_name.to_owned().into();
                    (
                        None,
                        Some(async move {
                            if let Some(service) = backend_transport {
                                let response = service.call_tool(request).await;
                                (name, Some(service), Some(response))
                            } else {
                                warn!("call_tool: trying to call a tool for which we have no backend");
                                (name, None, None)
                            }
                        }),
                    )
                } else {
                    (Some((name, backend_transport)), None)
                }
            })
            .unzip();

        let call_tool_tasks = call_tool_tasks.into_iter().flatten().collect::<Vec<_>>();
        if call_tool_tasks.len() > 1 {
            warn!("call_tool: More than one tool matching for tool name {}", request.name);

            session_manager.cleanup_backends("call_tool: invalid session.. duplicate tools detected").await;

            return Err(ErrorData {
                code: ErrorCode::INVALID_REQUEST,
                message: "Routing problem... session id not created".into(),
                data: None,
            });
        }

        let call_tool_tasks_results: Vec<(String, Option<_>, Option<_>)> = futures::future::join_all(call_tool_tasks).await;

        let (backend_services, responses): (Vec<_>, Vec<_>) = call_tool_tasks_results
            .into_iter()
            .map(|(name, service, response)| {
                info!("call_tool: backend {name} {response:?}");
                ((name.clone(), service), (name, response))
            })
            .unzip();

        session_manager.return_transports(backend_services.into_iter().chain(services.into_iter().flatten())).await;

        let responses = responses
            .into_iter()
            .filter_map(|(name, response)| if let Some(Ok(response)) = response { Some((name, response)) } else { None })
            .collect::<Vec<_>>();

        responses.first().cloned().map(|(_, r)| r).ok_or(ErrorData {
            code: ErrorCode::INTERNAL_ERROR,
            message: "Routing problem... got no responses from backends".into(),
            data: None,
        })
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        cx: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        let maybe_parts = cx.extensions.get::<Parts>();
        let maybe_session = maybe_parts.and_then(|parts| parts.extensions.get::<SessionId>());
        let maybe_user_config = maybe_parts.and_then(|parts| parts.extensions.get::<UserConfig>());
        info!("list_resources user_config = {maybe_user_config:#?} session_id = {maybe_session:#?}");
        Ok(ListResourcesResult {
            meta: None,
            resources: vec![
                RawResource {
                    uri: "test://static-text".into(),
                    name: "Static Text Resource".into(),
                    title: None,
                    description: Some("A static text resource for testing".into()),
                    mime_type: Some("text/plain".into()),
                    size: None,
                    icons: None,
                    meta: None,
                }
                .no_annotation(),
                RawResource {
                    uri: "test://static-binary".into(),
                    name: "Static Binary Resource".into(),
                    title: None,
                    description: Some("A static binary/blob resource for testing".into()),
                    mime_type: Some("image/png".into()),
                    size: None,
                    icons: None,
                    meta: None,
                }
                .no_annotation(),
            ],
            next_cursor: None,
        })
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        cx: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, ErrorData> {
        let maybe_parts = cx.extensions.get::<Parts>();
        let maybe_session = maybe_parts.and_then(|parts| parts.extensions.get::<SessionId>());
        let maybe_user_config = maybe_parts.and_then(|parts| parts.extensions.get::<UserConfig>());
        info!("read_resource user_config = {maybe_user_config:#?} session_id = {maybe_session:#?}");
        let uri = request.uri.as_str();
        match uri {
            "test://static-text" => Ok(ReadResourceResult::new(vec![ResourceContents::TextResourceContents {
                uri: uri.into(),
                mime_type: Some("text/plain".into()),
                text: "This is the content of the static text resource.".into(),
                meta: None,
            }])),
            "test://static-binary" => Ok(ReadResourceResult::new(vec![ResourceContents::BlobResourceContents {
                uri: uri.into(),
                mime_type: Some("image/png".into()),
                blob: TEST_IMAGE_DATA.into(),
                meta: None,
            }])),
            _ => {
                if uri.starts_with("test://template/") && uri.ends_with("/data") {
                    let id = uri.strip_prefix("test://template/").and_then(|s| s.strip_suffix("/data")).unwrap_or("unknown");
                    Ok(ReadResourceResult::new(vec![ResourceContents::TextResourceContents {
                        uri: uri.into(),
                        mime_type: Some("application/json".into()),
                        text: format!(r#"{{"id":"{id}","templateTest":true,"data":"Data for ID: {id}"}}"#),
                        meta: None,
                    }]))
                } else {
                    Err(ErrorData::resource_not_found(format!("Resource not found: {uri}"), None))
                }
            },
        }
    }

    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        cx: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, ErrorData> {
        let maybe_parts = cx.extensions.get::<Parts>();
        let maybe_session = maybe_parts.and_then(|parts| parts.extensions.get::<SessionId>());
        let maybe_user_config = maybe_parts.and_then(|parts| parts.extensions.get::<UserConfig>());
        info!("list_resource_templates user_config = {maybe_user_config:#?} session_id = {maybe_session:#?}");
        Ok(ListResourceTemplatesResult {
            meta: None,
            resource_templates: vec![
                RawResourceTemplate {
                    uri_template: "test://template/{id}/data".into(),
                    name: "Dynamic Resource".into(),
                    title: None,
                    description: Some("A dynamic resource with parameter substitution".into()),
                    mime_type: Some("application/json".into()),
                    icons: None,
                }
                .no_annotation(),
            ],
            next_cursor: None,
        })
    }

    async fn subscribe(&self, request: SubscribeRequestParams, cx: RequestContext<RoleServer>) -> Result<(), ErrorData> {
        let maybe_parts = cx.extensions.get::<Parts>();
        let maybe_session = maybe_parts.and_then(|parts| parts.extensions.get::<SessionId>());
        let maybe_user_config = maybe_parts.and_then(|parts| parts.extensions.get::<UserConfig>());
        info!("subscribe user_config = {maybe_user_config:#?} session_id = {maybe_session:#?}");

        let mut subs = self.subscriptions.lock().await;
        subs.insert(request.uri.clone());
        Ok(())
    }

    async fn unsubscribe(&self, request: UnsubscribeRequestParams, cx: RequestContext<RoleServer>) -> Result<(), ErrorData> {
        let maybe_parts = cx.extensions.get::<Parts>();
        let maybe_session = maybe_parts.and_then(|parts| parts.extensions.get::<SessionId>());
        let maybe_user_config = maybe_parts.and_then(|parts| parts.extensions.get::<UserConfig>());
        info!("unsubscribe user_config = {maybe_user_config:#?} session_id = {maybe_session:#?}");

        let mut subs = self.subscriptions.lock().await;
        subs.remove(request.uri.as_str());
        Ok(())
    }

    async fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        cx: RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, ErrorData> {
        let maybe_parts = cx.extensions.get::<Parts>();
        let maybe_session = maybe_parts.and_then(|parts| parts.extensions.get::<SessionId>());
        let maybe_user_config = maybe_parts.and_then(|parts| parts.extensions.get::<UserConfig>());
        info!("list_prompts user_config = {maybe_user_config:#?} session_id = {maybe_session:#?}");

        Ok(ListPromptsResult {
            meta: None,
            prompts: vec![
                Prompt::new("test_simple_prompt", Some("A simple test prompt with no arguments"), None),
                Prompt::new(
                    "test_prompt_with_arguments",
                    Some("A test prompt that accepts arguments"),
                    Some(vec![
                        PromptArgument::new("name").with_description("The name to greet").with_required(true),
                        PromptArgument::new("style").with_description("The greeting style").with_required(false),
                    ]),
                ),
                Prompt::new("test_prompt_with_embedded_resource", Some("A test prompt that includes an embedded resource"), None),
                Prompt::new("test_prompt_with_image", Some("A test prompt that includes an image"), None),
            ],
            next_cursor: None,
        })
    }

    async fn get_prompt(&self, request: GetPromptRequestParams, cx: RequestContext<RoleServer>) -> Result<GetPromptResult, ErrorData> {
        let maybe_parts = cx.extensions.get::<Parts>();
        let maybe_session = maybe_parts.and_then(|parts| parts.extensions.get::<SessionId>());
        let maybe_user_config = maybe_parts.and_then(|parts| parts.extensions.get::<UserConfig>());
        info!("get_prompt user_config = {maybe_user_config:#?} session_id = {maybe_session:#?}");
        match request.name.as_str() {
            "test_simple_prompt" => {
                Ok(GetPromptResult::new(vec![PromptMessage::new_text(PromptMessageRole::User, "This is a simple test prompt.")])
                    .with_description("A simple test prompt"))
            },
            "test_prompt_with_arguments" => {
                let args = request.arguments.unwrap_or_default();
                let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("World");
                let style = args.get("style").and_then(|v| v.as_str()).unwrap_or("friendly");
                Ok(GetPromptResult::new(vec![PromptMessage::new_text(
                    PromptMessageRole::User,
                    format!("Please greet {name} in a {style} style."),
                )])
                .with_description("A prompt with arguments"))
            },
            "test_prompt_with_embedded_resource" => Ok(GetPromptResult::new(vec![
                PromptMessage::new_text(PromptMessageRole::User, "Here is a resource:"),
                PromptMessage::new_resource(
                    PromptMessageRole::User,
                    "test://static-text".into(),
                    Some("text/plain".into()),
                    Some("Resource content for prompt".into()),
                    None,
                    None,
                    None,
                ),
            ])
            .with_description("A prompt with an embedded resource")),
            "test_prompt_with_image" => {
                let image_content = RawImageContent { data: TEST_IMAGE_DATA.into(), mime_type: "image/png".into(), meta: None };
                Ok(GetPromptResult::new(vec![
                    PromptMessage::new_text(PromptMessageRole::User, "Here is an image:"),
                    PromptMessage::new(PromptMessageRole::User, PromptMessageContent::Image { image: image_content.no_annotation() }),
                ])
                .with_description("A prompt with an image"))
            },
            _ => Err(ErrorData::invalid_params(format!("Unknown prompt: {}", request.name), None)),
        }
    }

    async fn complete(&self, request: CompleteRequestParams, cx: RequestContext<RoleServer>) -> Result<CompleteResult, ErrorData> {
        let maybe_parts = cx.extensions.get::<Parts>();
        let maybe_session = maybe_parts.and_then(|parts| parts.extensions.get::<SessionId>());
        let maybe_user_config = maybe_parts.and_then(|parts| parts.extensions.get::<UserConfig>());
        info!("complete user_config = {maybe_user_config:#?} session_id = {maybe_session:#?}");
        let values = match &request.r#ref {
            Reference::Resource(_) => {
                if request.argument.name == "id" {
                    vec!["1".into(), "2".into(), "3".into()]
                } else {
                    vec![]
                }
            },
            Reference::Prompt(prompt_ref) => {
                if request.argument.name == "name" {
                    vec!["Alice".into(), "Bob".into(), "Charlie".into()]
                } else if request.argument.name == "style" {
                    vec!["friendly".into(), "formal".into(), "casual".into()]
                } else {
                    vec![prompt_ref.name.clone()]
                }
            },
        };
        Ok(CompleteResult::new(CompletionInfo::new(values).map_err(|e| ErrorData::internal_error(e, None))?))
    }

    async fn set_level(&self, request: SetLevelRequestParams, cx: RequestContext<RoleServer>) -> Result<(), ErrorData> {
        let maybe_parts = cx.extensions.get::<Parts>();
        let maybe_session = maybe_parts.and_then(|parts| parts.extensions.get::<SessionId>());
        let maybe_user_config = maybe_parts.and_then(|parts| parts.extensions.get::<UserConfig>());
        info!("set_level user_config = {maybe_user_config:#?} session_id = {maybe_session:#?}");
        let mut level = self.log_level.lock().await;
        *level = request.level;
        Ok(())
    }
}

fn split_tool_name<'a>(tool_name: &'a std::borrow::Cow<'a, str>) -> Option<(&'a str, &'a str)> {
    tool_name.split_once('-')
}

const TEST_IMAGE_DATA: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8/5+hHgAHggJ/PchI7wAAAABJRU5ErkJggg==";
// Small base64-encoded WAV (silence)

fn merge_capabilities(_server_capabilities: Vec<(String, Option<ServerCapabilities>)>) -> ServerCapabilities {
    ServerCapabilities::builder().enable_prompts().enable_resources().enable_tools().enable_logging().build()
}

fn merge_tools(server_capabilities: Vec<(String, ListToolsResult)>) -> Vec<Tool> {
    server_capabilities
        .into_iter()
        .flat_map(|(backend_name, result)| {
            result
                .tools
                .into_iter()
                .map(|mut t| {
                    t.name = format!("{backend_name}-{}", t.name).into();
                    t
                })
                .collect::<Vec<_>>()
        })
        .sorted_by(|t, o| t.name.cmp(&o.name))
        .collect::<Vec<_>>()
}
