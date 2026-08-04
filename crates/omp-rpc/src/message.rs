use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use crate::{JsonObject, MessageAttribution, Usage};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextContent {
    pub text: String,
    pub text_signature: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageContent {
    pub data: String,
    pub mime_type: String,
    pub detail: Option<ImageDetail>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageDetail {
    Auto,
    Low,
    High,
    Original,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ContentType {
    Text,
    Image,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TextContentRef<'a> {
    #[serde(rename = "type")]
    kind: ContentType,
    text: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    text_signature: Option<&'a str>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TextContentWire {
    #[serde(rename = "type")]
    kind: ContentType,
    text: String,
    #[serde(default)]
    text_signature: Option<String>,
}

impl Serialize for TextContent {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        TextContentRef {
            kind: ContentType::Text,
            text: &self.text,
            text_signature: self.text_signature.as_deref(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for TextContent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = TextContentWire::deserialize(deserializer)?;
        if wire.kind != ContentType::Text {
            return Err(serde::de::Error::custom("text content must have type text"));
        }
        Ok(Self {
            text: wire.text,
            text_signature: wire.text_signature,
        })
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ImageContentRef<'a> {
    #[serde(rename = "type")]
    kind: ContentType,
    data: &'a str,
    mime_type: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<ImageDetail>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ImageContentWire {
    #[serde(rename = "type")]
    kind: ContentType,
    data: String,
    mime_type: String,
    #[serde(default)]
    detail: Option<ImageDetail>,
}

impl Serialize for ImageContent {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        ImageContentRef {
            kind: ContentType::Image,
            data: &self.data,
            mime_type: &self.mime_type,
            detail: self.detail,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ImageContent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ImageContentWire::deserialize(deserializer)?;
        if wire.kind != ContentType::Image {
            return Err(serde::de::Error::custom(
                "image content must have type image",
            ));
        }
        Ok(Self {
            data: wire.data,
            mime_type: wire.mime_type,
            detail: wire.detail,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum UserContentBlock {
    Text(TextContent),
    Image(ImageContent),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Blocks(Vec<UserContentBlock>),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ToolResultContent {
    Text(TextContent),
    Image(ImageContent),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "role")]
pub enum AgentMessage {
    #[serde(rename = "user")]
    User {
        content: MessageContent,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        synthetic: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        steering: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attribution: Option<MessageAttribution>,
        #[serde(
            default,
            rename = "providerPayload",
            skip_serializing_if = "Option::is_none"
        )]
        provider_payload: Option<Value>,
        timestamp: u64,
    },
    #[serde(rename = "developer")]
    Developer {
        content: MessageContent,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attribution: Option<MessageAttribution>,
        #[serde(
            default,
            rename = "providerPayload",
            skip_serializing_if = "Option::is_none"
        )]
        provider_payload: Option<Value>,
        timestamp: u64,
    },
    #[serde(rename = "assistant")]
    Assistant(Box<AssistantMessageBody>),
    #[serde(rename = "toolResult")]
    ToolResult(ToolResultMessageBody),
    #[serde(rename = "bashExecution", rename_all = "camelCase")]
    BashExecution {
        command: String,
        output: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exit_code: Option<i32>,
        cancelled: bool,
        truncated: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        meta: Option<Value>,
        timestamp: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exclude_from_context: Option<bool>,
    },
    #[serde(rename = "pythonExecution", rename_all = "camelCase")]
    PythonExecution {
        code: String,
        output: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exit_code: Option<i32>,
        cancelled: bool,
        truncated: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        meta: Option<Value>,
        timestamp: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exclude_from_context: Option<bool>,
    },
    #[serde(rename = "custom", rename_all = "camelCase")]
    Custom {
        custom_type: String,
        content: MessageContent,
        display: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        details: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attribution: Option<MessageAttribution>,
        timestamp: u64,
    },
    #[serde(rename = "hookMessage", rename_all = "camelCase")]
    HookMessage {
        custom_type: String,
        content: MessageContent,
        display: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        details: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attribution: Option<MessageAttribution>,
        timestamp: u64,
    },
    #[serde(rename = "branchSummary", rename_all = "camelCase")]
    BranchSummary {
        summary: String,
        from_id: String,
        timestamp: u64,
    },
    #[serde(rename = "compactionSummary", rename_all = "camelCase")]
    CompactionSummary {
        summary: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        short_summary: Option<String>,
        tokens_before: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_payload: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        blocks: Option<Vec<ToolResultContent>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        images: Option<Vec<ImageContent>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        warning: Option<String>,
        timestamp: u64,
    },
    #[serde(rename = "fileMention", rename_all = "camelCase")]
    FileMention {
        files: Vec<FileMention>,
        timestamp: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantMessageBody {
    pub content: Vec<AssistantContent>,
    pub api: String,
    pub provider: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_snapshot: Option<ContextSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_recovery: Option<AssistantRetryRecovery>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_provider: Option<String>,
    pub usage: Usage,
    pub stop_reason: StopReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_details: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_abort_messages: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_status: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled_features: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_payload: Option<Value>,
    pub timestamp: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttft: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantMessage {
    role: AssistantMessageRole,
    #[serde(flatten)]
    pub body: AssistantMessageBody,
}

impl AssistantMessage {
    #[must_use]
    pub fn new(body: AssistantMessageBody) -> Self {
        Self {
            role: AssistantMessageRole::Assistant,
            body,
        }
    }

    #[must_use]
    pub fn into_body(self) -> AssistantMessageBody {
        self.body
    }
}

impl From<AssistantMessageBody> for AssistantMessage {
    fn from(body: AssistantMessageBody) -> Self {
        Self::new(body)
    }
}

impl From<AssistantMessage> for AgentMessage {
    fn from(message: AssistantMessage) -> Self {
        Self::Assistant(Box::new(message.body))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum AssistantMessageRole {
    #[serde(rename = "assistant")]
    Assistant,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolResultMessageBody {
    pub tool_call_id: String,
    pub tool_name: String,
    pub content: Vec<ToolResultContent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
    pub is_error: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attribution: Option<MessageAttribution>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pruned_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_metadata: Option<ToolResultProviderMetadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub useless: Option<bool>,
    pub timestamp: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolResultMessage {
    role: ToolResultMessageRole,
    #[serde(flatten)]
    pub body: ToolResultMessageBody,
}

impl ToolResultMessage {
    #[must_use]
    pub fn new(body: ToolResultMessageBody) -> Self {
        Self {
            role: ToolResultMessageRole::ToolResult,
            body,
        }
    }

    #[must_use]
    pub fn into_body(self) -> ToolResultMessageBody {
        self.body
    }
}

impl From<ToolResultMessageBody> for ToolResultMessage {
    fn from(body: ToolResultMessageBody) -> Self {
        Self::new(body)
    }
}

impl From<ToolResultMessage> for AgentMessage {
    fn from(message: ToolResultMessage) -> Self {
        Self::ToolResult(message.body)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum ToolResultMessageRole {
    #[serde(rename = "toolResult")]
    ToolResult,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AssistantContent {
    #[serde(rename = "text", rename_all = "camelCase")]
    Text {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text_signature: Option<String>,
    },
    #[serde(rename = "thinking", rename_all = "camelCase")]
    Thinking {
        thinking: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thinking_signature: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        item_id: Option<String>,
    },
    #[serde(rename = "redactedThinking")]
    RedactedThinking { data: String },
    #[serde(rename = "fallback")]
    Fallback {
        from: FallbackModel,
        to: FallbackModel,
    },
    #[serde(rename = "anthropicServerTool")]
    AnthropicServerTool { block: Value },
    #[serde(rename = "image", rename_all = "camelCase")]
    Image {
        data: String,
        mime_type: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<ImageDetail>,
    },
    #[serde(rename = "toolCall")]
    ToolCall(ToolCall),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: JsonObject,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thought_signature: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_block: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_wire_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_metadata: Option<ToolCallProviderMetadata>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FallbackModel {
    pub model: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextSnapshot {
    pub prompt_tokens: u64,
    pub non_message_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message_timestamp: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantRetryRecovery {
    pub kind: AssistantRetryRecoveryKindMarker,
    pub status: AssistantRetryRecoveryStatus,
    pub attempt: u32,
    pub recovered_at: String,
    pub recovery: AssistantRecoveryKind,
    pub note: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<SupersedingResponse>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AssistantRetryRecoveryKindMarker {
    #[serde(rename = "auto-retry")]
    AutoRetry,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AssistantRetryRecoveryStatus {
    Recovered,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AssistantRecoveryKind {
    Credential,
    Model,
    Wait,
    Plain,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SupersedingResponse {
    pub timestamp: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_id: Option<String>,
    pub provider: String,
    pub model: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StopReason {
    Stop,
    Length,
    ToolUse,
    Error,
    Aborted,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileMention {
    pub path: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub byte_size: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped_reason: Option<FileMentionSkippedReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<ImageContent>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FileMentionSkippedReason {
    TooLarge,
    Binary,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ComputerAction {
    Click {
        button: ComputerMouseButton,
        x: f64,
        y: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        keys: Option<Vec<String>>,
    },
    DoubleClick {
        x: f64,
        y: f64,
        keys: Option<Vec<String>>,
    },
    Drag {
        path: Vec<ComputerPoint>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        keys: Option<Vec<String>>,
    },
    Keypress {
        keys: Vec<String>,
    },
    Move {
        x: f64,
        y: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        keys: Option<Vec<String>>,
    },
    Screenshot,
    Scroll {
        x: f64,
        y: f64,
        scroll_x: f64,
        scroll_y: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        keys: Option<Vec<String>>,
    },
    Type {
        text: String,
    },
    Wait,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ComputerMouseButton {
    Left,
    Right,
    Wheel,
    Back,
    Forward,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ComputerPoint {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerSafetyCheck {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ToolCallProviderMetadata {
    Computer {
        #[serde(rename = "providerItemId")]
        provider_item_id: String,
        actions: Vec<ComputerAction>,
        #[serde(rename = "pendingSafetyChecks")]
        pending_safety_checks: Vec<ComputerSafetyCheck>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ComputerScreenshotRef {
    ComputerScreenshot {
        #[serde(flatten)]
        location: ComputerScreenshotLocation,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ComputerScreenshotLocation {
    ImageUrl { image_url: String },
    FileId { file_id: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ToolResultProviderMetadata {
    Computer {
        screenshot: ComputerScreenshotRef,
        #[serde(rename = "acknowledgedSafetyChecks")]
        acknowledged_safety_checks: Vec<ComputerSafetyCheck>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum AssistantMessageEvent {
    Start {
        partial: Box<AssistantMessage>,
    },
    TextStart {
        content_index: usize,
        partial: Box<AssistantMessage>,
    },
    TextDelta {
        content_index: usize,
        delta: String,
        partial: Box<AssistantMessage>,
    },
    TextEnd {
        content_index: usize,
        content: String,
        partial: Box<AssistantMessage>,
    },
    ThinkingStart {
        content_index: usize,
        partial: Box<AssistantMessage>,
    },
    ThinkingDelta {
        content_index: usize,
        delta: String,
        partial: Box<AssistantMessage>,
    },
    ThinkingEnd {
        content_index: usize,
        content: String,
        partial: Box<AssistantMessage>,
    },
    ImageEnd {
        content_index: usize,
        content: ImageContent,
        partial: Box<AssistantMessage>,
    },
    #[serde(rename = "toolcall_start")]
    ToolCallStart {
        content_index: usize,
        partial: Box<AssistantMessage>,
    },
    #[serde(rename = "toolcall_delta")]
    ToolCallDelta {
        content_index: usize,
        delta: String,
        partial: Box<AssistantMessage>,
    },
    #[serde(rename = "toolcall_end")]
    ToolCallEnd {
        content_index: usize,
        tool_call: ToolCall,
        partial: Box<AssistantMessage>,
    },
    Done {
        reason: CompletedStopReason,
        message: Box<AssistantMessage>,
    },
    Error {
        reason: FailedStopReason,
        error: Box<AssistantMessage>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CompletedStopReason {
    Stop,
    Length,
    ToolUse,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FailedStopReason {
    Aborted,
    Error,
}
