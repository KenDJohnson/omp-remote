use serde::{Deserialize, Serialize};

use crate::{JsonObject, ToolLoadMode, ToolResultContent, ToolResultProviderMetadata};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostToolDefinition {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub description: String,
    pub parameters: JsonObject,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub load_mode: Option<ToolLoadMode>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostUriSchemeDefinition {
    pub scheme: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub writable: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub immutable: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentToolResult {
    pub content: Vec<ToolResultContent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_metadata: Option<ToolResultProviderMetadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub useless: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum HostRequestFrame {
    HostToolCall {
        id: String,
        tool_call_id: String,
        tool_name: String,
        arguments: JsonObject,
    },
    HostToolCancel {
        id: String,
        target_id: String,
    },
    HostUriRequest {
        id: String,
        operation: HostUriOperation,
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        content: Option<String>,
    },
    HostUriCancel {
        id: String,
        target_id: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum HostResponseFrame {
    HostToolUpdate {
        id: String,
        partial_result: AgentToolResult,
    },
    HostToolResult {
        id: String,
        result: AgentToolResult,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
    HostUriResult {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        content: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        content_type: Option<HostUriContentType>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        notes: Option<Vec<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        immutable: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HostUriOperation {
    Read,
    Write,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HostUriContentType {
    #[serde(rename = "text/markdown")]
    Markdown,
    #[serde(rename = "application/json")]
    Json,
    #[serde(rename = "text/plain")]
    PlainText,
}
