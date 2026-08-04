use std::{fmt, num::NonZeroU64};

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

#[derive(Debug)]
pub struct RpcFrameDecoder {
    pending: Option<PendingChunks>,
    max_reassembled_frame_bytes: NonZeroU64,
}

impl RpcFrameDecoder {
    #[must_use]
    pub fn new(max_reassembled_frame_bytes: NonZeroU64) -> Self {
        Self {
            pending: None,
            max_reassembled_frame_bytes,
        }
    }

    pub fn push_json_line(
        &mut self,
        line: impl AsRef<[u8]>,
    ) -> Result<Option<ServerMessage>, RpcFrameDecodeError> {
        let message =
            ServerMessage::from_json_line(line).map_err(RpcFrameDecodeError::InvalidJson)?;
        self.push(message)
    }

    pub fn push(
        &mut self,
        message: ServerMessage,
    ) -> Result<Option<ServerMessage>, RpcFrameDecodeError> {
        let ServerMessage::Transport(TransportFrame::RpcChunk { chunk }) = message else {
            if self.pending.is_some() {
                return Err(RpcFrameDecodeError::SequenceInterrupted);
            }
            if let ServerMessage::Transport(TransportFrame::Ready { ready }) = &message {
                self.max_reassembled_frame_bytes = ready.max_reassembled_frame_bytes();
            }
            return Ok(Some(message));
        };

        if chunk.byte_length() > self.max_reassembled_frame_bytes {
            return Err(RpcFrameDecodeError::DeclaredLengthExceedsLimit);
        }

        if self.pending.is_none() {
            if chunk.index() != 0 {
                return Err(RpcFrameDecodeError::SequenceMustStartAtZero);
            }
            self.pending = Some(PendingChunks {
                chunk_id: chunk.chunk_id().to_owned(),
                count: chunk.count(),
                byte_length: chunk.byte_length(),
                next_index: 0,
                data: Vec::with_capacity(
                    usize::try_from(chunk.byte_length().get())
                        .expect("RPC reassembly limit fits supported targets"),
                ),
            });
        }

        let pending = self
            .pending
            .as_mut()
            .expect("RPC chunk state was initialized");
        if pending.chunk_id != chunk.chunk_id()
            || pending.count != chunk.count()
            || pending.byte_length != chunk.byte_length()
            || pending.next_index != chunk.index()
        {
            return Err(RpcFrameDecodeError::SequenceMismatch);
        }

        chunk.append_decoded_data(&mut pending.data);
        pending.next_index += 1;
        if pending.data.len() as u64 > pending.byte_length.get() {
            return Err(RpcFrameDecodeError::SequenceExceedsDeclaredLength);
        }
        if u64::from(pending.next_index) < pending.count.get() {
            return Ok(None);
        }
        if pending.data.len() as u64 != pending.byte_length.get() {
            return Err(RpcFrameDecodeError::SequenceLengthMismatch);
        }

        let pending = self
            .pending
            .take()
            .expect("completed RPC chunk state is present");
        let json = String::from_utf8(pending.data).map_err(RpcFrameDecodeError::InvalidUtf8)?;
        let message =
            ServerMessage::from_json_line(json).map_err(RpcFrameDecodeError::InvalidJson)?;
        if matches!(
            &message,
            ServerMessage::Transport(TransportFrame::RpcChunk { .. })
        ) {
            return Err(RpcFrameDecodeError::NestedChunkFrame);
        }
        Ok(Some(message))
    }

    #[must_use]
    pub fn is_reassembling(&self) -> bool {
        self.pending.is_some()
    }
}

impl Default for RpcFrameDecoder {
    fn default() -> Self {
        Self::new(
            NonZeroU64::new(crate::ReadyFrame::DEFAULT_MAX_REASSEMBLED_FRAME_BYTES)
                .expect("the default RPC reassembly limit is non-zero"),
        )
    }
}

#[derive(Debug)]
struct PendingChunks {
    chunk_id: String,
    count: NonZeroU64,
    byte_length: NonZeroU64,
    next_index: u32,
    data: Vec<u8>,
}

#[derive(Debug)]
pub enum RpcFrameDecodeError {
    SequenceInterrupted,
    SequenceMustStartAtZero,
    SequenceMismatch,
    SequenceExceedsDeclaredLength,
    SequenceLengthMismatch,
    DeclaredLengthExceedsLimit,
    InvalidUtf8(std::string::FromUtf8Error),
    InvalidJson(serde_json::Error),
    NestedChunkFrame,
}

impl fmt::Display for RpcFrameDecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SequenceInterrupted => formatter.write_str("RPC chunk sequence was interrupted"),
            Self::SequenceMustStartAtZero => {
                formatter.write_str("RPC chunk sequence must start at index zero")
            }
            Self::SequenceMismatch => formatter.write_str("RPC chunk sequence metadata mismatch"),
            Self::SequenceExceedsDeclaredLength => {
                formatter.write_str("RPC chunk sequence exceeds its declared byte length")
            }
            Self::SequenceLengthMismatch => {
                formatter.write_str("RPC chunk sequence byte length mismatch")
            }
            Self::DeclaredLengthExceedsLimit => {
                formatter.write_str("RPC chunk sequence exceeds the advertised reassembly limit")
            }
            Self::InvalidUtf8(_) => formatter.write_str("reassembled RPC frame is not valid UTF-8"),
            Self::InvalidJson(_) => formatter.write_str("RPC frame is not valid JSON"),
            Self::NestedChunkFrame => {
                formatter.write_str("reassembled RPC frame cannot contain another chunk frame")
            }
        }
    }
}

impl std::error::Error for RpcFrameDecodeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidUtf8(error) => Some(error),
            Self::InvalidJson(error) => Some(error),
            _ => None,
        }
    }
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
