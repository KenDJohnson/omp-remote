use std::collections::{BTreeMap, VecDeque};

use omp_control_client::{ClientEvent, ConnectionStatus, ReplicatedState};
use omp_control_protocol::{
    AgentId, AgentSnapshot, DeviceId, DeviceSummary, EventEnvelope, EventSequence,
    UiInteractionEnvelope,
};
use omp_rpc::{
    AgentMessage, AssistantContent, AssistantMessageEvent, MessageContent, ServerMessage,
    SessionEvent, SideChannelFrame, UserContentBlock,
};

const MAX_TRANSCRIPT_ENTRIES: usize = 1_000;
const MAX_PARTIAL_TEXT_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug)]
pub struct AppModel {
    pub connection: ConnectionStatus,
    pub own_device_id: Option<DeviceId>,
    pub agents: BTreeMap<AgentId, AgentSnapshot>,
    pub selected_agent: Option<AgentId>,
    pub transcripts: BTreeMap<AgentId, Transcript>,
    pub interactions: BTreeMap<AgentId, PendingInteraction>,
    pub devices: Vec<DeviceSummary>,
    pub error: Option<String>,
}

impl Default for AppModel {
    fn default() -> Self {
        Self {
            connection: ConnectionStatus::Disconnected { reason: None },
            own_device_id: None,
            agents: BTreeMap::new(),
            selected_agent: None,
            transcripts: BTreeMap::new(),
            interactions: BTreeMap::new(),
            devices: Vec::new(),
            error: None,
        }
    }
}

impl AppModel {
    pub fn apply_client_event(&mut self, event: ClientEvent, state: &ReplicatedState) {
        match event {
            ClientEvent::ConnectionChanged(status) => {
                if let ConnectionStatus::Connected { device_id, .. } = &status {
                    self.own_device_id = Some(device_id.clone());
                }
                self.connection = status;
            }
            ClientEvent::StateChanged(agent_id) => {
                if let Some(agent) = state.agent(&agent_id) {
                    self.agents.insert(agent_id.clone(), agent.clone());
                    self.selected_agent.get_or_insert(agent_id);
                }
            }
            ClientEvent::AgentEvent(event) => self.apply_agent_event(event),
            ClientEvent::InteractionRequest(interaction) => {
                self.interactions.insert(
                    interaction.agent_id.clone(),
                    PendingInteraction {
                        envelope: interaction,
                        lease: InteractionLeaseState::Acquiring,
                    },
                );
            }
            ClientEvent::ResyncRequired(agent_id) => {
                self.error = Some(format!("{agent_id} requires a fresh state snapshot"));
            }
            ClientEvent::ProtocolError(error) => {
                self.error = Some(error.message);
            }
            ClientEvent::ReplicationError(error) => {
                self.error = Some(error.to_string());
            }
        }
    }

    pub fn replace_devices(&mut self, devices: Vec<DeviceSummary>) {
        self.devices = devices;
    }

    pub fn replace_agents(&mut self, agents: Vec<AgentSnapshot>) {
        for agent in agents {
            let agent_id = agent.agent_id.clone();
            self.agents.insert(agent_id.clone(), agent);
            self.selected_agent.get_or_insert(agent_id);
        }
    }

    pub fn set_interaction_lease(&mut self, agent_id: &AgentId, state: InteractionLeaseState) {
        if let Some(interaction) = self.interactions.get_mut(agent_id) {
            interaction.lease = state;
        }
    }

    pub fn dismiss_interaction(&mut self, agent_id: &AgentId) {
        self.interactions.remove(agent_id);
    }

    fn apply_agent_event(&mut self, envelope: EventEnvelope) {
        let transcript = self.transcripts.entry(envelope.agent_id).or_default();
        transcript.apply(envelope.event_sequence, envelope.event);
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PendingInteraction {
    pub envelope: UiInteractionEnvelope,
    pub lease: InteractionLeaseState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InteractionLeaseState {
    Acquiring,
    Owned,
    Unavailable(String),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Transcript {
    pub entries: VecDeque<TranscriptEntry>,
    pub partial_assistant: String,
    pub partial_thinking: String,
}

impl Transcript {
    fn apply(&mut self, sequence: EventSequence, message: ServerMessage) {
        match message {
            ServerMessage::SessionEvent(SessionEvent::MessageStart { message }) => {
                if matches!(*message, AgentMessage::User { .. }) {
                    self.push(
                        sequence,
                        TranscriptKind::User,
                        render_agent_message(&message),
                    );
                }
            }
            ServerMessage::SessionEvent(SessionEvent::MessageUpdate {
                assistant_message_event,
                ..
            }) => match *assistant_message_event {
                AssistantMessageEvent::TextDelta { delta, .. } => {
                    append_bounded(&mut self.partial_assistant, &delta);
                }
                AssistantMessageEvent::ThinkingDelta { delta, .. } => {
                    append_bounded(&mut self.partial_thinking, &delta);
                }
                _ => {}
            },
            ServerMessage::SessionEvent(SessionEvent::MessageEnd { message }) => {
                if matches!(*message, AgentMessage::Assistant(_)) {
                    if !self.partial_thinking.is_empty() {
                        let text = std::mem::take(&mut self.partial_thinking);
                        self.push(sequence, TranscriptKind::Thinking, text);
                    }
                    let text = if self.partial_assistant.is_empty() {
                        render_agent_message(&message)
                    } else {
                        std::mem::take(&mut self.partial_assistant)
                    };
                    if !text.is_empty() {
                        self.push(sequence, TranscriptKind::Assistant, text);
                    }
                }
            }
            ServerMessage::SessionEvent(SessionEvent::ToolExecutionStart {
                tool_name,
                args,
                intent,
                ..
            }) => {
                let intent = intent.map_or_else(String::new, |value| format!(" — {value}"));
                self.push(
                    sequence,
                    TranscriptKind::Tool,
                    format!("{tool_name}{intent}\n{args}"),
                );
            }
            ServerMessage::SessionEvent(SessionEvent::ToolExecutionEnd {
                tool_name,
                result,
                is_error,
                ..
            }) => {
                let prefix = if is_error == Some(true) {
                    "Tool failed"
                } else {
                    "Tool finished"
                };
                self.push(
                    sequence,
                    TranscriptKind::Tool,
                    format!("{prefix}: {tool_name}\n{result}"),
                );
            }
            ServerMessage::SessionEvent(SessionEvent::Notice { level, message, .. }) => {
                self.push(
                    sequence,
                    TranscriptKind::System,
                    format!("{level:?}: {message}"),
                );
            }
            ServerMessage::SideChannel(SideChannelFrame::CommandOutput { text }) => {
                self.push(sequence, TranscriptKind::System, text);
            }
            _ => {}
        }
    }

    fn push(&mut self, sequence: EventSequence, kind: TranscriptKind, text: String) {
        if self.entries.len() == MAX_TRANSCRIPT_ENTRIES {
            self.entries.pop_front();
        }
        self.entries.push_back(TranscriptEntry {
            sequence,
            kind,
            text,
        });
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TranscriptEntry {
    pub sequence: EventSequence,
    pub kind: TranscriptKind,
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TranscriptKind {
    User,
    Assistant,
    Thinking,
    Tool,
    System,
}

fn append_bounded(target: &mut String, delta: &str) {
    let remaining = MAX_PARTIAL_TEXT_BYTES.saturating_sub(target.len());
    if remaining == 0 {
        return;
    }
    let boundary = floor_char_boundary(delta, remaining.min(delta.len()));
    target.push_str(&delta[..boundary]);
}

fn floor_char_boundary(value: &str, mut index: usize) -> usize {
    while !value.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn render_agent_message(message: &AgentMessage) -> String {
    match message {
        AgentMessage::User { content, .. } | AgentMessage::Developer { content, .. } => {
            render_message_content(content)
        }
        AgentMessage::Assistant(message) => message
            .content
            .iter()
            .filter_map(|content| match content {
                AssistantContent::Text { text, .. } => Some(text.as_str()),
                AssistantContent::Thinking { thinking, .. } => Some(thinking.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        AgentMessage::BashExecution {
            command, output, ..
        } => format!("$ {command}\n{output}"),
        AgentMessage::PythonExecution { code, output, .. } => format!("{code}\n{output}"),
        _ => String::new(),
    }
}

fn render_message_content(content: &MessageContent) -> String {
    match content {
        MessageContent::Text(text) => text.clone(),
        MessageContent::Blocks(blocks) => blocks
            .iter()
            .filter_map(|block| match block {
                UserContentBlock::Text(text) => Some(text.text.as_str()),
                UserContentBlock::Image(_) => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

#[cfg(test)]
mod tests {
    use omp_control_protocol::{EventEnvelope, EventSequence};
    use omp_rpc::{ServerMessage, SessionEvent};
    use serde_json::json;

    use super::*;

    fn event(agent_id: &AgentId, sequence: u64, event: SessionEvent) -> EventEnvelope {
        EventEnvelope {
            agent_id: agent_id.clone(),
            event_sequence: EventSequence(sequence),
            event: ServerMessage::SessionEvent(event),
        }
    }

    #[test]
    fn transcript_renders_tool_events_in_sequence_order() {
        let agent_id = AgentId::new("agent-1").unwrap();
        let mut model = AppModel::default();
        model.apply_agent_event(event(
            &agent_id,
            1,
            SessionEvent::ToolExecutionStart {
                tool_call_id: "tool-1".to_owned(),
                tool_name: "read".to_owned(),
                args: json!({"path": "PLAN.md"}),
                intent: Some("Read plan".to_owned()),
            },
        ));
        model.apply_agent_event(event(
            &agent_id,
            2,
            SessionEvent::ToolExecutionEnd {
                tool_call_id: "tool-1".to_owned(),
                tool_name: "read".to_owned(),
                result: json!("done"),
                is_error: Some(false),
            },
        ));

        let transcript = model.transcripts.get(&agent_id).unwrap();
        assert_eq!(transcript.entries[0].sequence, EventSequence(1));
        assert_eq!(transcript.entries[1].sequence, EventSequence(2));
        assert_eq!(transcript.entries.len(), 2);
        assert!(transcript.entries[0].text.contains("read"));
        assert!(transcript.entries[1].text.contains("done"));
    }

    #[test]
    fn replacing_agents_selects_the_first_authoritative_agent() {
        let first = AgentSnapshot::initial(AgentId::new("first").unwrap());
        let second = AgentSnapshot::initial(AgentId::new("second").unwrap());
        let mut model = AppModel::default();

        model.replace_agents(vec![first.clone(), second]);

        assert_eq!(model.selected_agent.as_ref(), Some(&first.agent_id));
        assert_eq!(model.agents.len(), 2);
    }
}
