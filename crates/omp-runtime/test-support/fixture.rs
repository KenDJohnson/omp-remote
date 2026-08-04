use std::io::{self, BufRead, BufWriter, Write};

use serde_json::{Value, json};

fn main() -> io::Result<()> {
    let stdin = io::stdin();
    let mut stdout = BufWriter::new(io::stdout().lock());
    write_frame(
        &mut stdout,
        &json!({
            "type": "ready",
            "protocolVersion": 1,
            "supportedProtocolVersions": [1, 2],
            "maxFrameBytes": 1_048_576,
            "maxReassembledFrameBytes": 67_108_864
        }),
    )?;

    let mut delayed_steer = None;
    for line in stdin.lock().lines() {
        let frame: Value = serde_json::from_str(&line?)?;
        let request_id = frame.get("id").cloned().unwrap_or(Value::Null);
        match frame.get("type").and_then(Value::as_str) {
            Some("negotiate_protocol") => write_frame(
                &mut stdout,
                &success(
                    request_id,
                    "negotiate_protocol",
                    Some(json!({ "protocolVersion": 2 })),
                ),
            )?,
            Some("steer") => delayed_steer = Some(request_id),
            Some("follow_up") => {
                write_frame(&mut stdout, &success(request_id, "follow_up", None))?;
                if let Some(steer_id) = delayed_steer.take() {
                    write_frame(&mut stdout, &success(steer_id, "steer", None))?;
                }
            }
            Some("prompt") => match frame.get("message").and_then(Value::as_str) {
                Some("exit") => {
                    write_frame(
                        &mut stdout,
                        &success(request_id, "prompt", Some(json!({ "agentInvoked": false }))),
                    )?;
                    std::process::exit(7);
                }
                Some("agent") => {
                    write_frame(
                        &mut stdout,
                        &success(request_id, "prompt", Some(json!({ "agentInvoked": true }))),
                    )?;
                    write_frame(&mut stdout, &json!({ "type": "agent_start" }))?;
                    write_frame(&mut stdout, &json!({ "type": "agent_end", "messages": [] }))?;
                }
                Some("interaction") => {
                    write_frame(
                        &mut stdout,
                        &success(request_id, "prompt", Some(json!({ "agentInvoked": true }))),
                    )?;
                    write_frame(
                        &mut stdout,
                        &json!({
                            "type": "extension_ui_request",
                            "id": "fixture-ui-1",
                            "method": "select",
                            "title": "Choose a fixture option",
                            "options": ["Alpha", "Beta"]
                        }),
                    )?;
                }
                Some("late-local") => {
                    write_frame(&mut stdout, &success(request_id.clone(), "prompt", None))?;
                    write_frame(
                        &mut stdout,
                        &json!({
                            "type": "prompt_result",
                            "id": request_id,
                            "agentInvoked": false
                        }),
                    )?;
                }
                Some("fail") => write_frame(
                    &mut stdout,
                    &json!({
                        "id": request_id,
                        "type": "response",
                        "command": "prompt",
                        "success": false,
                        "error": "fixture rejected prompt"
                    }),
                )?,
                _ => write_frame(
                    &mut stdout,
                    &success(request_id, "prompt", Some(json!({ "agentInvoked": false }))),
                )?,
            },
            Some("extension_ui_response") => {
                write_frame(&mut stdout, &json!({ "type": "agent_end", "messages": [] }))?;
            }
            Some("abort") => write_frame(&mut stdout, &success(request_id, "abort", None))?,
            Some(command) => write_frame(
                &mut stdout,
                &json!({
                    "id": request_id,
                    "type": "response",
                    "command": command,
                    "success": false,
                    "error": "unsupported fixture command"
                }),
            )?,
            None => write_frame(
                &mut stdout,
                &json!({
                    "type": "response",
                    "command": "parse",
                    "success": false,
                    "error": "missing command type"
                }),
            )?,
        }
    }
    Ok(())
}

fn success(id: Value, command: &str, data: Option<Value>) -> Value {
    let mut response = json!({
        "id": id,
        "type": "response",
        "command": command,
        "success": true
    });
    if let Some(data) = data {
        response["data"] = data;
    }
    response
}

fn write_frame(stdout: &mut BufWriter<impl Write>, frame: &Value) -> io::Result<()> {
    serde_json::to_writer(&mut *stdout, frame)?;
    stdout.write_all(b"\n")?;
    stdout.flush()
}
