use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Effort {
    Minimal,
    Low,
    Medium,
    High,
    XHigh,
    Max,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThinkingLevel {
    Inherit,
    Off,
    Minimal,
    Low,
    Medium,
    High,
    XHigh,
    Max,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConfiguredThinkingLevel {
    Auto,
    Inherit,
    Off,
    Minimal,
    Low,
    Medium,
    High,
    XHigh,
    Max,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub total_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orchestration: Option<OrchestrationUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub premium_requests: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cttl: Option<CacheWriteTtlUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server: Option<ServerToolUsage>,
    pub cost: UsageCost,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrchestrationUsage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheWriteTtlUsage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ephemeral5m: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ephemeral1h: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerToolUsage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub web_search: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub web_fetch: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageCost {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
    pub total: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelInput {
    Text,
    Image,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ThinkingControlMode {
    Effort,
    Budget,
    GoogleLevel,
    AnthropicAdaptive,
    AnthropicBudgetEffort,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThinkingConfig {
    pub mode: ThinkingControlMode,
    pub efforts: Vec<Effort>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_level: Option<Effort>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort_map: Option<BTreeMap<Effort, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_display: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort_routing: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort_budgets: Option<BTreeMap<Effort, u64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suppress_when_off: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires_effort: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCost {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Model {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_model_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_mode: Option<ReasoningMode>,
    pub name: String,
    pub api: String,
    pub provider: String,
    pub base_url: String,
    pub reasoning: bool,
    pub input: Vec<ModelInput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_input_decoder: Option<ImageInputDecoder>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_tools: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_computer_use: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_computer_use_config: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gitlab_duo_workflow_root_namespace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor_max_mode: Option<bool>,
    pub cost: ModelCost,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub premium_multiplier: Option<f64>,
    pub context_window: Option<u64>,
    pub max_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub omit_max_output_tokens: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport: Option<ModelTransport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefer_websockets: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub use_responses_lite: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_promotion_target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compaction_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_compaction: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<ThinkingConfig>,
    pub compat: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compat_config: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub apply_patch_tool_type: Option<ApplyPatchToolType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_o_auth: Option<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningMode {
    Pro,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageInputDecoder {
    Stb,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ModelTransport {
    PiNative,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApplyPatchToolType {
    Freeform,
    Function,
}
