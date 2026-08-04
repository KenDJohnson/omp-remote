use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    AgentMessage, ConfiguredThinkingLevel, ContextUsage, Effort, InterruptMode, JsonObject,
    MessageAttribution, MessageContent, Model, QueueMode, StructuredSubagentSchemaMode,
    ThinkingLevel, ToolResultContent,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
    Abandoned,
    Blocked,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoItem {
    pub content: String,
    pub status: TodoStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocker: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoPhase {
    pub name: String,
    pub tasks: Vec<TodoItem>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<Model>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_level: Option<ThinkingLevel>,
    pub is_streaming: bool,
    pub is_compacting: bool,
    pub steering_mode: QueueMode,
    pub follow_up_mode: QueueMode,
    pub interrupt_mode: InterruptMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_file: Option<String>,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_name: Option<String>,
    pub auto_compaction_enabled: bool,
    pub message_count: u64,
    pub queued_message_count: u64,
    pub todo_phases: Vec<TodoPhase>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dump_tools: Option<Vec<DumpTool>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_usage: Option<ContextUsage>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DumpTool {
    pub name: String,
    pub description: String,
    pub parameters: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub examples: Option<Vec<Value>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStats {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_file: Option<String>,
    pub session_id: String,
    pub user_messages: u64,
    pub assistant_messages: u64,
    pub tool_calls: u64,
    pub tool_results: u64,
    pub total_messages: u64,
    pub tokens: SessionTokenStats,
    pub premium_requests: f64,
    pub cost: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_usage: Option<ContextUsage>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionTokenStats {
    pub input: u64,
    pub output: u64,
    pub reasoning: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub total: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactionResult {
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short_summary: Option<String>,
    pub first_kept_entry_id: String,
    pub tokens_before: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preserve_data: Option<JsonObject>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BashResult {
    pub output: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    pub cancelled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timed_out: Option<bool>,
    pub truncated: bool,
    pub total_lines: u64,
    pub total_bytes: u64,
    pub output_lines: u64,
    pub output_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceTierByFamily {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openai: Option<ServiceTier>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anthropic: Option<ServiceTier>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub google: Option<ServiceTier>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ServiceTier {
    Auto,
    Default,
    Flex,
    Scale,
    Priority,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum FileEntry {
    #[serde(rename = "session", rename_all = "camelCase")]
    Session {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        version: Option<u32>,
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title_source: Option<SessionTitleSource>,
        timestamp: String,
        cwd: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        additional_directories: Option<Vec<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_session: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_prompt_cache_key: Option<String>,
    },
    #[serde(rename = "message")]
    Message {
        #[serde(flatten)]
        base: SessionEntryBase,
        message: Box<AgentMessage>,
    },
    #[serde(rename = "thinking_level_change", rename_all = "camelCase")]
    ThinkingLevelChange {
        #[serde(flatten)]
        base: SessionEntryBase,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thinking_level: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        configured: Option<String>,
    },
    #[serde(rename = "model_change")]
    ModelChange {
        #[serde(flatten)]
        base: SessionEntryBase,
        model: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        role: Option<String>,
    },
    #[serde(rename = "service_tier_change", rename_all = "camelCase")]
    ServiceTierChange {
        #[serde(flatten)]
        base: SessionEntryBase,
        service_tier: Option<ServiceTierByFamily>,
    },
    #[serde(rename = "compaction", rename_all = "camelCase")]
    Compaction {
        #[serde(flatten)]
        base: SessionEntryBase,
        summary: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        short_summary: Option<String>,
        first_kept_entry_id: String,
        tokens_before: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        details: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        preserve_data: Option<JsonObject>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from_extension: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        warning: Option<String>,
    },
    #[serde(rename = "branch_summary", rename_all = "camelCase")]
    BranchSummary {
        #[serde(flatten)]
        base: SessionEntryBase,
        from_id: String,
        summary: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        details: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from_extension: Option<bool>,
    },
    #[serde(rename = "custom", rename_all = "camelCase")]
    Custom {
        #[serde(flatten)]
        base: SessionEntryBase,
        custom_type: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        data: Option<Value>,
    },
    #[serde(rename = "custom_message", rename_all = "camelCase")]
    CustomMessage {
        #[serde(flatten)]
        base: SessionEntryBase,
        custom_type: String,
        content: MessageContent,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        details: Option<Value>,
        display: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attribution: Option<MessageAttribution>,
    },
    #[serde(rename = "label", rename_all = "camelCase")]
    Label {
        #[serde(flatten)]
        base: SessionEntryBase,
        target_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    #[serde(rename = "title_change", rename_all = "camelCase")]
    TitleChange {
        #[serde(flatten)]
        base: SessionEntryBase,
        title: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        previous_title: Option<String>,
        source: SessionTitleSource,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trigger: Option<String>,
    },
    #[serde(rename = "ttsr_injection", rename_all = "camelCase")]
    TtsrInjection {
        #[serde(flatten)]
        base: SessionEntryBase,
        injected_rules: Vec<String>,
    },
    #[serde(rename = "session_init", rename_all = "camelCase")]
    SessionInit {
        #[serde(flatten)]
        base: SessionEntryBase,
        system_prompt: String,
        task: String,
        tools: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output_schema: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output_schema_mode: Option<StructuredSubagentSchemaMode>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        restrict_tool_names: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        spawns: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        read_summarize: Option<bool>,
    },
    #[serde(rename = "mode_change")]
    ModeChange {
        #[serde(flatten)]
        base: SessionEntryBase,
        mode: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        data: Option<BTreeMap<String, Value>>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionEntryBase {
    pub id: String,
    pub parent_id: Option<String>,
    pub timestamp: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionTitleSource {
    Auto,
    User,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagesPage {
    pub messages: Vec<AgentMessage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    pub total_messages: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchMessage {
    pub entry_id: String,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextUsageBreakdown {
    pub context_window: u64,
    pub anchored: bool,
    pub used_tokens: u64,
    pub system_prompt_tokens: u64,
    pub system_tools_tokens: u64,
    pub system_context_tokens: u64,
    pub skills_tokens: u64,
    pub messages_tokens: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallSummary {
    pub name: String,
    pub arguments: JsonObject,
    pub content: Vec<ToolResultContent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_level: Option<ConfiguredThinkingLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<Effort>,
}
