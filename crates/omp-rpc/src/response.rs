use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

use crate::{
    AgentMessage, BashResult, BranchMessage, CompactionResult, Effort, MessagesPage, Model,
    RequestId, SessionState, SessionStats, SubagentMessagesResult, SubagentSnapshot,
    SubagentSubscriptionLevel, TodoPhase,
};

#[derive(Clone, Debug, PartialEq)]
pub enum Response {
    Success {
        id: Option<RequestId>,
        result: Box<SuccessResponse>,
    },
    Error {
        id: Option<RequestId>,
        command: String,
        error: String,
        code: Option<String>,
    },
}

impl Response {
    #[must_use]
    pub fn success(id: Option<RequestId>, result: SuccessResponse) -> Self {
        Self::Success {
            id,
            result: Box::new(result),
        }
    }

    #[must_use]
    pub fn error(
        id: Option<RequestId>,
        command: impl Into<String>,
        error: impl Into<String>,
    ) -> Self {
        Self::Error {
            id,
            command: command.into(),
            error: error.into(),
            code: None,
        }
    }

    #[must_use]
    pub fn id(&self) -> Option<&RequestId> {
        match self {
            Self::Success { id, .. } | Self::Error { id, .. } => id.as_ref(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "command",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum SuccessResponse {
    NegotiateProtocol {
        data: NegotiatedProtocol,
    },
    Prompt {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        data: Option<PromptAcknowledgement>,
    },
    Steer,
    FollowUp,
    Abort,
    AbortAndPrompt,
    NewSession {
        data: CancelledResult,
    },
    GetState {
        data: SessionState,
    },
    GetAvailableCommands {
        data: AvailableCommands,
    },
    SetTodos {
        data: SetTodosResult,
    },
    SetHostTools {
        data: SetHostToolsResult,
    },
    SetHostUriSchemes {
        data: SetHostUriSchemesResult,
    },
    SetSubagentSubscription {
        data: SetSubagentSubscriptionResult,
    },
    GetSubagents {
        data: SubagentsResult,
    },
    GetSubagentMessages {
        data: SubagentMessagesResult,
    },
    SetModel {
        data: Model,
    },
    CycleModel {
        data: Option<ModelCycleResult>,
    },
    GetAvailableModels {
        data: AvailableModels,
    },
    SetThinkingLevel,
    CycleThinkingLevel {
        data: Option<ThinkingLevelCycleResult>,
    },
    SetSteeringMode,
    SetFollowUpMode,
    SetInterruptMode,
    Compact {
        data: CompactionResult,
    },
    SetAutoCompaction,
    SetAutoRetry,
    AbortRetry,
    Bash {
        data: BashResult,
    },
    AbortBash,
    GetSessionStats {
        data: SessionStats,
    },
    ExportHtml {
        data: ExportHtmlResult,
    },
    SwitchSession {
        data: CancelledResult,
    },
    Branch {
        data: BranchResult,
    },
    GetBranchMessages {
        data: BranchMessagesResult,
    },
    GetLastAssistantText {
        data: LastAssistantTextResult,
    },
    SetSessionName,
    Handoff {
        data: Option<HandoffResult>,
    },
    GetMessages {
        data: MessagesResult,
    },
    GetMessagesPage {
        data: MessagesPage,
    },
    GetLoginProviders {
        data: LoginProvidersResult,
    },
    Login {
        data: LoginResult,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NegotiatedProtocol {
    pub protocol_version: crate::ProtocolV2,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptAcknowledgement {
    pub agent_invoked: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelledResult {
    pub cancelled: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AvailableCommands {
    pub commands: Vec<AvailableSlashCommand>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AvailableSlashCommand {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aliases: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<SlashCommandInput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subcommands: Option<Vec<SlashSubcommand>>,
    pub source: SlashCommandSource,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlashCommandInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlashSubcommand {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SlashCommandSource {
    Builtin,
    Skill,
    Extension,
    Custom,
    McpPrompt,
    File,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetTodosResult {
    pub todo_phases: Vec<TodoPhase>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetHostToolsResult {
    pub tool_names: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetHostUriSchemesResult {
    pub schemes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetSubagentSubscriptionResult {
    pub level: SubagentSubscriptionLevel,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SubagentsResult {
    pub subagents: Vec<SubagentSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCycleResult {
    pub model: Model,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_level: Option<crate::ThinkingLevel>,
    pub is_scoped: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AvailableModels {
    pub models: Vec<Model>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThinkingLevelCycleResult {
    pub level: Effort,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportHtmlResult {
    pub path: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchResult {
    pub text: String,
    pub cancelled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchMessagesResult {
    pub messages: Vec<BranchMessage>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastAssistantTextResult {
    pub text: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HandoffResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saved_path: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MessagesResult {
    pub messages: Vec<AgentMessage>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginProvidersResult {
    pub providers: Vec<LoginProvider>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginProvider {
    pub id: String,
    pub name: String,
    pub available: bool,
    pub authenticated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginResult {
    pub provider_id: String,
}

#[derive(Serialize)]
struct SuccessResponseRef<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<&'a RequestId>,
    #[serde(rename = "type")]
    kind: ResponseKind,
    success: bool,
    #[serde(flatten)]
    result: &'a SuccessResponse,
}

#[derive(Deserialize)]
struct SuccessResponseWire {
    #[serde(default)]
    id: Option<RequestId>,
    #[serde(rename = "type")]
    kind: ResponseKind,
    success: bool,
    #[serde(flatten)]
    result: Box<SuccessResponse>,
}

#[derive(Serialize)]
struct ErrorResponseRef<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<&'a RequestId>,
    #[serde(rename = "type")]
    kind: ResponseKind,
    command: &'a str,
    success: bool,
    error: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<&'a str>,
}

#[derive(Deserialize)]
struct ErrorResponseWire {
    #[serde(default)]
    id: Option<RequestId>,
    #[serde(rename = "type")]
    kind: ResponseKind,
    command: String,
    success: bool,
    error: String,
    #[serde(default)]
    code: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum ResponseKind {
    #[serde(rename = "response")]
    Response,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ResponseWire {
    Error(ErrorResponseWire),
    Success(Box<SuccessResponseWire>),
}

impl Serialize for Response {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Success { id, result } => SuccessResponseRef {
                id: id.as_ref(),
                kind: ResponseKind::Response,
                success: true,
                result,
            }
            .serialize(serializer),
            Self::Error {
                id,
                command,
                error,
                code,
            } => ErrorResponseRef {
                id: id.as_ref(),
                kind: ResponseKind::Response,
                command,
                success: false,
                error,
                code: code.as_deref(),
            }
            .serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for Response {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        match ResponseWire::deserialize(deserializer)? {
            ResponseWire::Success(wire) if wire.success => {
                let _ = wire.kind;
                Ok(Self::Success {
                    id: wire.id,
                    result: wire.result,
                })
            }
            ResponseWire::Success(_) => Err(de::Error::custom(
                "successful RPC response must set success to true",
            )),
            ResponseWire::Error(wire) if !wire.success => {
                let _ = wire.kind;
                Ok(Self::Error {
                    id: wire.id,
                    command: wire.command,
                    error: wire.error,
                    code: wire.code,
                })
            }
            ResponseWire::Error(_) => Err(de::Error::custom(
                "failed RPC response must set success to false",
            )),
        }
    }
}
