use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, de};

use crate::{
    Effort, HostToolDefinition, HostUriSchemeDefinition, ImageContent, InterruptMode, ProtocolV2,
    QueueMode, RequestId, StreamingBehavior, SubagentSubscriptionLevel, ThinkingLevel, TodoPhase,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Command {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<RequestId>,
    #[serde(flatten)]
    pub kind: CommandKind,
}

impl Command {
    #[must_use]
    pub fn new(kind: CommandKind) -> Self {
        Self { id: None, kind }
    }

    #[must_use]
    pub fn with_id(id: impl Into<RequestId>, kind: CommandKind) -> Self {
        Self {
            id: Some(id.into()),
            kind,
        }
    }

    #[must_use]
    pub fn negotiate_protocol(id: impl Into<RequestId>) -> Self {
        Self::with_id(
            id,
            CommandKind::NegotiateProtocol {
                protocol_version: ProtocolV2,
            },
        )
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum CommandKind {
    NegotiateProtocol {
        protocol_version: ProtocolV2,
    },
    Prompt {
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        images: Option<Vec<ImageContent>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        streaming_behavior: Option<StreamingBehavior>,
    },
    Steer {
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        images: Option<Vec<ImageContent>>,
    },
    FollowUp {
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        images: Option<Vec<ImageContent>>,
    },
    Abort,
    AbortAndPrompt {
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        images: Option<Vec<ImageContent>>,
    },
    NewSession {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_session: Option<String>,
    },
    GetState,
    GetAvailableCommands,
    SetTodos {
        phases: Vec<TodoPhase>,
    },
    SetHostTools {
        tools: Vec<HostToolDefinition>,
    },
    SetHostUriSchemes {
        schemes: Vec<HostUriSchemeDefinition>,
    },
    SetSubagentSubscription {
        level: SubagentSubscriptionLevel,
    },
    GetSubagents,
    GetSubagentMessages {
        #[serde(flatten)]
        selector: SubagentTranscriptSelector,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from_byte: Option<u64>,
    },
    SetModel {
        provider: String,
        model_id: String,
    },
    CycleModel,
    GetAvailableModels,
    SetThinkingLevel {
        level: ThinkingLevel,
    },
    CycleThinkingLevel,
    SetSteeringMode {
        mode: QueueMode,
    },
    SetFollowUpMode {
        mode: QueueMode,
    },
    SetInterruptMode {
        mode: InterruptMode,
    },
    Compact {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        custom_instructions: Option<String>,
    },
    SetAutoCompaction {
        enabled: bool,
    },
    SetAutoRetry {
        enabled: bool,
    },
    AbortRetry,
    Bash {
        command: String,
    },
    AbortBash,
    GetSessionStats,
    ExportHtml {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output_path: Option<String>,
    },
    SwitchSession {
        session_path: String,
    },
    Branch {
        entry_id: String,
    },
    GetBranchMessages,
    GetLastAssistantText,
    SetSessionName {
        name: String,
    },
    Handoff {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        custom_instructions: Option<String>,
    },
    GetMessages,
    GetMessagesPage {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cursor: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<MessagePageLimit>,
    },
    GetLoginProviders,
    Login {
        provider_id: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SubagentTranscriptSelector {
    SubagentId {
        #[serde(rename = "subagentId")]
        subagent_id: String,
    },
    SessionFile {
        #[serde(rename = "sessionFile")]
        session_file: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct MessagePageLimit(u16);

impl MessagePageLimit {
    pub const MIN: u16 = 1;
    pub const MAX: u16 = 256;

    pub fn new(value: u16) -> Result<Self, MessagePageLimitError> {
        if (Self::MIN..=Self::MAX).contains(&value) {
            Ok(Self(value))
        } else {
            Err(MessagePageLimitError(value))
        }
    }

    #[must_use]
    pub fn get(self) -> u16 {
        self.0
    }
}

impl TryFrom<u16> for MessagePageLimit {
    type Error = MessagePageLimitError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl<'de> Deserialize<'de> for MessagePageLimit {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(u16::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MessagePageLimitError(u16);

impl fmt::Display for MessagePageLimitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "RPC message page limit must be between {} and {}, got {}",
            MessagePageLimit::MIN,
            MessagePageLimit::MAX,
            self.0
        )
    }
}

impl std::error::Error for MessagePageLimitError {}

impl From<Effort> for ThinkingLevel {
    fn from(effort: Effort) -> Self {
        match effort {
            Effort::Minimal => Self::Minimal,
            Effort::Low => Self::Low,
            Effort::Medium => Self::Medium,
            Effort::High => Self::High,
            Effort::XHigh => Self::XHigh,
            Effort::Max => Self::Max,
        }
    }
}
