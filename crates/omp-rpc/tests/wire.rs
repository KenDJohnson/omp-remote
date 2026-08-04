use std::num::NonZeroU64;

use omp_rpc::*;
use serde_json::json;

fn round_trip<T>(value: &T)
where
    T: std::fmt::Debug + PartialEq + serde::Serialize + serde::de::DeserializeOwned,
{
    let encoded = serde_json::to_vec(value).expect("serialize wire value");
    let decoded = serde_json::from_slice(&encoded).expect("deserialize wire value");
    assert_eq!(*value, decoded);
}

#[test]
fn every_command_variant_round_trips() {
    let commands = vec![
        CommandKind::NegotiateProtocol {
            protocol_version: ProtocolV2,
        },
        CommandKind::Prompt {
            message: "hello".into(),
            images: Some(vec![ImageContent {
                data: "aGVsbG8=".into(),
                mime_type: "image/png".into(),
                detail: Some(ImageDetail::Original),
            }]),
            streaming_behavior: Some(StreamingBehavior::FollowUp),
        },
        CommandKind::Steer {
            message: "steer".into(),
            images: None,
        },
        CommandKind::FollowUp {
            message: "later".into(),
            images: None,
        },
        CommandKind::Abort,
        CommandKind::AbortAndPrompt {
            message: "restart".into(),
            images: None,
        },
        CommandKind::NewSession {
            parent_session: Some("parent.jsonl".into()),
        },
        CommandKind::GetState,
        CommandKind::GetAvailableCommands,
        CommandKind::SetTodos {
            phases: vec![TodoPhase {
                name: "Build".into(),
                tasks: vec![TodoItem {
                    content: "Type the protocol".into(),
                    status: TodoStatus::InProgress,
                    blocker: None,
                }],
            }],
        },
        CommandKind::SetHostTools {
            tools: vec![HostToolDefinition {
                name: "echo".into(),
                label: Some("Echo".into()),
                description: "Echo input".into(),
                parameters: JsonObject::new(),
                hidden: None,
                load_mode: Some(ToolLoadMode::Discoverable),
            }],
        },
        CommandKind::SetHostUriSchemes {
            schemes: vec![HostUriSchemeDefinition {
                scheme: "db".into(),
                description: None,
                writable: Some(true),
                immutable: Some(false),
            }],
        },
        CommandKind::SetSubagentSubscription {
            level: SubagentSubscriptionLevel::Events,
        },
        CommandKind::GetSubagents,
        CommandKind::GetSubagentMessages {
            selector: SubagentTranscriptSelector::SubagentId {
                subagent_id: "child-1".into(),
            },
            from_byte: Some(42),
        },
        CommandKind::SetModel {
            provider: "openai".into(),
            model_id: "gpt-test".into(),
        },
        CommandKind::CycleModel,
        CommandKind::GetAvailableModels,
        CommandKind::SetThinkingLevel {
            level: ThinkingLevel::High,
        },
        CommandKind::CycleThinkingLevel,
        CommandKind::SetSteeringMode {
            mode: QueueMode::All,
        },
        CommandKind::SetFollowUpMode {
            mode: QueueMode::OneAtATime,
        },
        CommandKind::SetInterruptMode {
            mode: InterruptMode::Wait,
        },
        CommandKind::Compact {
            custom_instructions: Some("preserve decisions".into()),
        },
        CommandKind::SetAutoCompaction { enabled: false },
        CommandKind::SetAutoRetry { enabled: true },
        CommandKind::AbortRetry,
        CommandKind::Bash {
            command: "pwd".into(),
        },
        CommandKind::AbortBash,
        CommandKind::GetSessionStats,
        CommandKind::ExportHtml {
            output_path: Some("session.html".into()),
        },
        CommandKind::SwitchSession {
            session_path: "session.jsonl".into(),
        },
        CommandKind::Branch {
            entry_id: "entry-1".into(),
        },
        CommandKind::GetBranchMessages,
        CommandKind::GetLastAssistantText,
        CommandKind::SetSessionName {
            name: "typed RPC".into(),
        },
        CommandKind::Handoff {
            custom_instructions: None,
        },
        CommandKind::GetMessages,
        CommandKind::GetMessagesPage {
            cursor: Some("opaque".into()),
            limit: Some(MessagePageLimit::new(256).unwrap()),
        },
        CommandKind::GetLoginProviders,
        CommandKind::Login {
            provider_id: "anthropic".into(),
        },
    ];

    assert_eq!(commands.len(), 41);
    for kind in commands {
        round_trip(&Command::with_id("request-1", kind));
    }
}

#[test]
fn prompt_uses_the_canonical_wire_shape() {
    let command = Command::with_id(
        "req_1",
        CommandKind::Prompt {
            message: "Summarize this repo".into(),
            images: None,
            streaming_behavior: Some(StreamingBehavior::FollowUp),
        },
    );

    assert_eq!(
        serde_json::to_value(command).unwrap(),
        json!({
            "id": "req_1",
            "type": "prompt",
            "message": "Summarize this repo",
            "streamingBehavior": "followUp"
        })
    );
}

#[test]
fn protocol_negotiation_only_accepts_v2() {
    let error = serde_json::from_value::<Command>(json!({
        "id": "protocol-1",
        "type": "negotiate_protocol",
        "protocolVersion": 1
    }))
    .unwrap_err();

    assert!(error.to_string().contains("protocol version 2"));
}

#[test]
fn success_and_error_responses_keep_their_invariants() {
    let success = Response::success(
        Some("req_1".into()),
        SuccessResponse::Prompt {
            data: Some(PromptAcknowledgement {
                agent_invoked: false,
            }),
        },
    );
    let success_json = json!({
        "id": "req_1",
        "type": "response",
        "command": "prompt",
        "success": true,
        "data": { "agentInvoked": false }
    });
    assert_eq!(serde_json::to_value(&success).unwrap(), success_json);
    assert_eq!(
        serde_json::from_value::<Response>(success_json).unwrap(),
        success
    );

    let error_json = json!({
        "id": "req_2",
        "type": "response",
        "command": "prompt",
        "success": false,
        "error": "prompt failed",
        "code": "session_busy"
    });
    let error = serde_json::from_value::<Response>(error_json.clone()).unwrap();
    assert_eq!(serde_json::to_value(error).unwrap(), error_json);

    assert!(
        serde_json::from_value::<Response>(json!({
            "type": "response",
            "command": "abort",
            "success": false
        }))
        .is_err()
    );
}

#[test]
fn ready_and_chunk_frames_validate_transport_invariants() {
    let ready_json = json!({
        "type": "ready",
        "protocolVersion": 1,
        "supportedProtocolVersions": [1, 2],
        "maxFrameBytes": 1048576,
        "maxReassembledFrameBytes": 67108864
    });
    let ready = ServerMessage::from_json_line(ready_json.to_string()).unwrap();
    assert_eq!(serde_json::to_value(&ready).unwrap(), ready_json);

    let legacy_json = json!({ "type": "ready" });
    let legacy = ServerMessage::from_json_line(legacy_json.to_string()).unwrap();
    assert!(matches!(
        &legacy,
        ServerMessage::Transport(TransportFrame::Ready { ready })
            if !ready.advertises_capabilities()
    ));
    assert_eq!(serde_json::to_value(legacy).unwrap(), legacy_json);

    let chunk = ChunkFrame::new(
        "rpc-1",
        0,
        NonZeroU64::new(4).unwrap(),
        NonZeroU64::new(ChunkFrame::MIN_BYTE_LENGTH).unwrap(),
        "eyJ0eXBlIg==",
    )
    .unwrap();
    round_trip(&TransportFrame::RpcChunk { chunk });

    assert!(
        serde_json::from_value::<TransportFrame>(json!({
            "type": "rpc_chunk",
            "chunkId": "rpc-1",
            "index": 2,
            "count": 2,
            "byteLength": 1048576,
            "data": "e30="
        }))
        .is_err()
    );
    assert!(
        ChunkFrame::new(
            "rpc-1",
            0,
            NonZeroU64::new(4).unwrap(),
            NonZeroU64::new(ChunkFrame::MIN_BYTE_LENGTH).unwrap(),
            "not base64",
        )
        .is_err()
    );
    assert!(
        ChunkFrame::new(
            "rpc-1",
            0,
            NonZeroU64::new(4).unwrap(),
            NonZeroU64::new(ChunkFrame::MIN_BYTE_LENGTH - 1).unwrap(),
            "e30=",
        )
        .is_err()
    );
}

#[test]
fn client_and_server_jsonl_helpers_cover_side_channels() {
    let client = ClientMessage::Host(HostResponseFrame::HostUriResult {
        id: "uri_1".into(),
        content: Some("name=Alice\n".into()),
        content_type: Some(HostUriContentType::PlainText),
        notes: Some(vec!["fresh".into()]),
        immutable: Some(false),
        is_error: None,
        error: None,
    });
    let encoded = client.to_json_line().unwrap();
    assert_eq!(encoded.last(), Some(&b'\n'));
    assert_eq!(ClientMessage::from_json_line(encoded).unwrap(), client);

    let server = ServerMessage::SideChannel(SideChannelFrame::PromptResult {
        id: Some("req_1".into()),
        agent_invoked: false,
    });
    let encoded = server.to_json_line().unwrap();
    assert_eq!(ServerMessage::from_json_line(encoded).unwrap(), server);

    let overflow = json!({
        "type": "rpc_frame_error",
        "originalType": "message_end",
        "error": "RPC frame exceeded the transport limit"
    });
    let decoded = ServerMessage::from_json_line(overflow.to_string()).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), overflow);
}

#[test]
fn extension_cancellation_cannot_encode_false() {
    let response = ExtensionUiResponse::Cancelled {
        timed_out: Some(true),
    };
    assert_eq!(
        serde_json::to_value(response).unwrap(),
        json!({ "cancelled": true, "timedOut": true })
    );
    assert!(serde_json::from_value::<ExtensionUiResponse>(json!({ "cancelled": false })).is_err());
}

#[test]
fn content_blocks_carry_their_wire_discriminants() {
    let image = ImageContent {
        data: "aGVsbG8=".into(),
        mime_type: "image/png".into(),
        detail: None,
    };
    assert_eq!(
        serde_json::to_value(image).unwrap(),
        json!({ "type": "image", "data": "aGVsbG8=", "mimeType": "image/png" })
    );

    let text = TextContent {
        text: "working".into(),
        text_signature: None,
    };
    assert_eq!(
        serde_json::to_value(text).unwrap(),
        json!({ "type": "text", "text": "working" })
    );
}

#[test]
fn agent_message_newtype_variants_flatten_into_the_wire_object() {
    let assistant_value = json!({
        "role": "assistant",
        "content": [{ "type": "text", "text": "done" }],
        "api": "openai-responses",
        "provider": "openai",
        "model": "gpt-test",
        "usage": {
            "input": 1,
            "output": 2,
            "cacheRead": 0,
            "cacheWrite": 0,
            "totalTokens": 3,
            "cost": {
                "input": 0.0,
                "output": 0.0,
                "cacheRead": 0.0,
                "cacheWrite": 0.0,
                "total": 0.0
            }
        },
        "stopReason": "stop",
        "timestamp": 1
    });
    let assistant = serde_json::from_value::<AgentMessage>(assistant_value.clone()).unwrap();
    assert_eq!(serde_json::to_value(assistant).unwrap(), assistant_value);
    let assistant = serde_json::from_value::<AssistantMessage>(assistant_value.clone()).unwrap();
    assert_eq!(serde_json::to_value(assistant).unwrap(), assistant_value);

    let tool_result_value = json!({
        "role": "toolResult",
        "toolCallId": "call-1",
        "toolName": "read",
        "content": [{ "type": "text", "text": "result" }],
        "isError": false,
        "timestamp": 2
    });
    let tool_result = serde_json::from_value::<AgentMessage>(tool_result_value.clone()).unwrap();
    assert_eq!(
        serde_json::to_value(tool_result).unwrap(),
        tool_result_value
    );
    let tool_result =
        serde_json::from_value::<ToolResultMessage>(tool_result_value.clone()).unwrap();
    assert_eq!(
        serde_json::to_value(tool_result).unwrap(),
        tool_result_value
    );
}

#[test]
fn message_page_limit_enforces_the_server_range() {
    assert_eq!(MessagePageLimit::new(1).unwrap().get(), 1);
    assert_eq!(MessagePageLimit::new(256).unwrap().get(), 256);
    assert!(MessagePageLimit::new(0).is_err());
    assert!(MessagePageLimit::new(257).is_err());
    assert!(serde_json::from_value::<MessagePageLimit>(json!(257)).is_err());
}

#[test]
fn todo_state_matches_current_omp_source_shape() {
    let value = json!({
        "name": "Evaluation",
        "tasks": [{
            "content": "Map the read tool surface",
            "status": "in_progress"
        }]
    });
    let phase = serde_json::from_value::<TodoPhase>(value.clone()).unwrap();
    assert_eq!(serde_json::to_value(phase).unwrap(), value);
}

#[test]
fn unknown_top_level_frames_are_rejected_instead_of_silently_dropped() {
    let error = ServerMessage::from_json_line(
        serde_json::to_vec(&json!({ "type": "future_frame", "payload": {} })).unwrap(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("did not match any variant"));
}
