use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    AgentMessage, AssistantMessage, AssistantMessageEvent, Command, CompactionResult,
    ConfiguredThinkingLevel, Effort, ExtensionUiRequestFrame, ExtensionUiResponseFrame,
    HostRequestFrame, HostResponseFrame, Model, RequestId, Response, SubagentFrame, ThinkingLevel,
    TodoItem, ToolResultMessage,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ClientMessage {
    Command(Command),
    ExtensionUi(ExtensionUiResponseFrame),
    Host(HostResponseFrame),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ServerMessage {
    Response(Response),
    Transport(TransportFrame),
    ExtensionUi(ExtensionUiRequestFrame),
    Host(HostRequestFrame),
    SideChannel(SideChannelFrame),
    Subagent(SubagentFrame),
    SessionEvent(SessionEvent),
}

impl ClientMessage {
    pub fn from_json_line(line: impl AsRef<[u8]>) -> serde_json::Result<Self> {
        serde_json::from_slice(line.as_ref())
    }

    pub fn to_json_line(&self) -> serde_json::Result<Vec<u8>> {
        encode_json_line(self)
    }
}

impl ServerMessage {
    pub fn from_json_line(line: impl AsRef<[u8]>) -> serde_json::Result<Self> {
        serde_json::from_slice(line.as_ref())
    }

    pub fn to_json_line(&self) -> serde_json::Result<Vec<u8>> {
        encode_json_line(self)
    }
}

fn encode_json_line(value: &impl Serialize) -> serde_json::Result<Vec<u8>> {
    let mut output = Vec::new();
    serde_json::to_writer(&mut output, value)?;
    output.push(b'\n');
    Ok(output)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TransportFrame {
    Ready {
        #[serde(flatten)]
        ready: crate::ReadyFrame,
    },
    RpcChunk {
        #[serde(flatten)]
        chunk: crate::ChunkFrame,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum SideChannelFrame {
    ExtensionError {
        extension_path: String,
        event: String,
        error: String,
    },
    AvailableCommandsUpdate {
        commands: Vec<crate::AvailableSlashCommand>,
    },
    PromptResult {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<RequestId>,
        agent_invoked: bool,
    },
    CommandOutput {
        text: String,
    },
    SessionInfoUpdate {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        session_id: String,
    },
    ConfigUpdate {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<Box<Model>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thinking_level: Option<ThinkingLevel>,
    },
    RpcFrameError {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        original_type: Option<String>,
        error: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum SessionEvent {
    AgentStart,
    AgentEnd {
        messages: Vec<AgentMessage>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message_count: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        telemetry: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        coverage: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        is_terminal: Option<bool>,
    },
    TurnStart,
    TurnEnd {
        message: Box<AssistantMessage>,
        tool_results: Vec<ToolResultMessage>,
    },
    MessageStart {
        message: Box<AgentMessage>,
    },
    MessageUpdate {
        message: Box<AgentMessage>,
        assistant_message_event: Box<AssistantMessageEvent>,
    },
    MessageEnd {
        message: Box<AgentMessage>,
    },
    ToolExecutionStart {
        tool_call_id: String,
        tool_name: String,
        args: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        intent: Option<String>,
    },
    ToolExecutionUpdate {
        tool_call_id: String,
        tool_name: String,
        args: Value,
        partial_result: Value,
    },
    ToolExecutionEnd {
        tool_call_id: String,
        tool_name: String,
        result: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
    AutoCompactionStart {
        reason: AutoCompactionReason,
        action: CompactionAction,
    },
    AutoCompactionEnd {
        action: CompactionAction,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        result: Option<CompactionResult>,
        aborted: bool,
        will_retry: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error_message: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        skipped: Option<bool>,
    },
    AutoRetryStart {
        attempt: u32,
        max_attempts: u32,
        delay_ms: u64,
        error_message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error_id: Option<u64>,
    },
    AutoRetryEnd {
        success: bool,
        attempt: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        final_error: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        recovered_errors: Option<Vec<Value>>,
    },
    RetryFallbackApplied {
        from: String,
        to: String,
        role: String,
    },
    RetryFallbackSucceeded {
        model: String,
        role: String,
    },
    TtsrTriggered {
        rules: Vec<Value>,
    },
    TodoReminder {
        todos: Vec<TodoItem>,
        attempt: u32,
        max_attempts: u32,
    },
    TodoAutoClear,
    IrcMessage {
        message: AgentMessage,
    },
    Notice {
        level: NoticeLevel,
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source: Option<String>,
    },
    ThinkingLevelChanged {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thinking_level: Option<ThinkingLevel>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        configured: Option<ConfiguredThinkingLevel>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resolved: Option<Effort>,
    },
    GoalUpdated {
        #[serde(default)]
        goal: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        state: Option<Value>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NoticeLevel {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AutoCompactionReason {
    Threshold,
    Overflow,
    Idle,
    Incomplete,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CompactionAction {
    ContextFull,
    Handoff,
    Shake,
    Snapcompact,
}
