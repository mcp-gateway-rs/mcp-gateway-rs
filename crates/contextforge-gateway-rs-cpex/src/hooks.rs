use std::any::Any;

use rmcp::model::CallToolRequestParams;
use serde_json::{Map, Value};

pub type RuntimeHookError = Box<dyn std::error::Error + Send + Sync + 'static>;
pub type RuntimeHookState = Box<dyn Any + Send + Sync + 'static>;

#[derive(Debug)]
pub enum ToolArgumentsUpdate {
    Unchanged,
    Replace(Option<Map<String, Value>>),
}

impl ToolArgumentsUpdate {
    pub fn apply_to_request(self, request: &mut CallToolRequestParams, routed_tool_name: &str) {
        request.name = routed_tool_name.to_owned().into();
        if let Self::Replace(arguments) = self {
            request.arguments = arguments;
        }
    }
}

pub struct ToolPreCallResult {
    pub arguments: ToolArgumentsUpdate,
    pub state: Option<RuntimeHookState>,
}

impl ToolPreCallResult {
    pub fn unchanged() -> Self {
        Self { arguments: ToolArgumentsUpdate::Unchanged, state: None }
    }
}
