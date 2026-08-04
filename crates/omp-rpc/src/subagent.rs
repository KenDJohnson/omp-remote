use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    AgentMessage, AgentSource, FileEntry, SessionEvent, StructuredSubagentSchemaMode, Usage,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentSnapshot {
    pub id: String,
    pub index: u32,
    pub agent: String,
    pub agent_source: AgentSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub status: AgentProgressStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignment: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_file: Option<String>,
    pub last_update: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<AgentProgress>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_tool_call_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentMessagesResult {
    pub session_file: String,
    pub from_byte: u64,
    pub next_byte: u64,
    pub reset: bool,
    pub entries: Vec<FileEntry>,
    pub messages: Vec<AgentMessage>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentProgressStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Aborted,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentProgress {
    pub index: u32,
    pub id: String,
    pub agent: String,
    pub agent_source: AgentSource,
    pub status: AgentProgressStatus,
    pub task: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignment: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_intent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_tool: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_tool_args: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_tool_start_ms: Option<u64>,
    pub recent_tools: Vec<RecentTool>,
    pub recent_output: Vec<String>,
    pub tool_count: u64,
    pub requests: u64,
    pub tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    pub cost: f64,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_override: Option<ModelOverride>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_model_is_fallback: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extracted_tool_data: Option<BTreeMap<String, Vec<Value>>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_state: Option<RetryState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_failure: Option<RetryFailure>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inflight_task_details: Option<Box<TaskToolDetails>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentTool {
    pub tool: String,
    pub args: String,
    #[serde(rename = "endMs")]
    pub end_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ModelOverride {
    One(String),
    Many(Vec<String>),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetryState {
    pub attempt: u32,
    pub max_attempts: u32,
    pub delay_ms: u64,
    pub error_message: String,
    pub started_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetryFailure {
    pub attempt: u32,
    pub error_message: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SingleSubagentResult {
    pub index: u32,
    pub id: String,
    pub agent: String,
    pub agent_source: AgentSource,
    pub task: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignment: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_intent: Option<String>,
    pub exit_code: i32,
    pub output: String,
    pub stderr: String,
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_output: Option<StructuredSubagentOutput>,
    pub duration_ms: u64,
    pub tokens: u64,
    pub requests: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_override: Option<ModelOverride>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_model_is_fallback: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aborted: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub abort_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patch_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_base_sha: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nested_patches: Option<Vec<Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extracted_tool_data: Option<BTreeMap<String, Vec<Value>>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_failure: Option<RetryFailure>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_meta: Option<SubagentOutputMeta>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskToolDetails {
    pub project_agents_dir: Option<String>,
    pub results: Vec<SingleSubagentResult>,
    pub total_duration_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_paths: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<Vec<AgentProgress>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#async: Option<AsyncTaskState>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AsyncTaskState {
    pub state: AsyncTaskStatus,
    #[serde(rename = "jobId")]
    pub job_id: String,
    pub r#type: AsyncTaskType,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AsyncTaskStatus {
    Running,
    Completed,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AsyncTaskType {
    Task,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentOutputMeta {
    pub line_count: u64,
    pub char_count: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StructuredSubagentOutput {
    pub source: StructuredSubagentSchemaSource,
    pub mode: StructuredSubagentSchemaMode,
    pub status: StructuredSubagentValidationStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StructuredSubagentSchemaSource {
    Caller,
    Agent,
    Session,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StructuredSubagentValidationStatus {
    Valid,
    Invalid,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentProgressPayload {
    pub index: u32,
    pub agent: String,
    pub agent_source: AgentSource,
    pub task: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignment: Option<String>,
    pub progress: AgentProgress,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detached: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SubagentEventPayload {
    pub id: String,
    pub event: SessionEvent,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentLifecyclePayload {
    pub id: String,
    pub agent: String,
    pub agent_source: AgentSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub status: SubagentLifecycleStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_tool_call_id: Option<String>,
    pub index: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detached: Option<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SubagentLifecycleStatus {
    Started,
    Completed,
    Failed,
    Aborted,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SubagentFrame {
    SubagentLifecycle {
        payload: Box<SubagentLifecyclePayload>,
    },
    SubagentProgress {
        payload: Box<SubagentProgressPayload>,
    },
    SubagentEvent {
        payload: Box<SubagentEventPayload>,
    },
}
