use std::time::Duration;

use omp_rpc::{CommandKind, Response, ServerMessage, SessionEvent, SuccessResponse};
use omp_runtime::{
    OmpRuntime, PromptCompletion, PromptPhase, RuntimeConfig, RuntimeEvent, RuntimeStatus,
};
use tokio::time;

fn fixture_config() -> RuntimeConfig {
    RuntimeConfig::new(env!("CARGO_BIN_EXE_omp-runtime-fixture"))
        .startup_timeout(Duration::from_secs(2))
        .request_timeout(Duration::from_secs(2))
        .shutdown_timeout(Duration::from_secs(2))
}

#[tokio::test]
async fn correlates_out_of_order_responses_by_request_id() {
    let runtime = OmpRuntime::spawn(fixture_config()).await.unwrap();
    assert!(matches!(
        *runtime.status().borrow(),
        RuntimeStatus::Running { .. }
    ));

    let steer_runtime = runtime.clone();
    let steer = tokio::spawn(async move {
        steer_runtime
            .request(CommandKind::Steer {
                message: "first".into(),
                images: None,
            })
            .await
    });
    time::sleep(Duration::from_millis(20)).await;
    let follow_up = runtime
        .request(CommandKind::FollowUp {
            message: "second".into(),
            images: None,
        })
        .await
        .unwrap();
    let steer = steer.await.unwrap().unwrap();

    assert!(matches!(
        follow_up,
        Response::Success { result, .. } if matches!(*result, SuccessResponse::FollowUp)
    ));
    assert!(matches!(
        steer,
        Response::Success { result, .. } if matches!(*result, SuccessResponse::Steer)
    ));

    let exit = runtime.shutdown().await.unwrap();
    assert!(exit.success);
    assert!(!exit.forced);
}

#[tokio::test]
async fn delivers_events_and_tracks_agent_prompt_completion() {
    let runtime = OmpRuntime::spawn(fixture_config()).await.unwrap();
    let mut events = runtime.events();
    let mut prompt_status = runtime.prompt_status();

    let response = runtime.prompt("agent", None, None).await.unwrap();
    assert!(matches!(response, Response::Success { .. }));

    time::timeout(Duration::from_secs(2), async {
        loop {
            if matches!(
                prompt_status.borrow().as_ref().map(|status| &status.phase),
                Some(PromptPhase::Completed(PromptCompletion::Agent))
            ) {
                break;
            }
            prompt_status.changed().await.unwrap();
        }
    })
    .await
    .unwrap();

    let mut saw_start = false;
    time::timeout(Duration::from_secs(2), async {
        loop {
            match events.recv().await.unwrap() {
                RuntimeEvent::Frame(ServerMessage::SessionEvent(SessionEvent::AgentStart)) => {
                    saw_start = true;
                }
                RuntimeEvent::Frame(ServerMessage::SessionEvent(SessionEvent::AgentEnd {
                    ..
                })) => break,
                _ => {}
            }
        }
    })
    .await
    .unwrap();
    assert!(saw_start);

    runtime.shutdown().await.unwrap();
}

#[tokio::test]
async fn completes_local_prompts_from_the_late_prompt_result_hint() {
    let runtime = OmpRuntime::spawn(fixture_config()).await.unwrap();
    let mut prompt_status = runtime.prompt_status();

    runtime.prompt("late-local", None, None).await.unwrap();
    time::timeout(Duration::from_secs(2), async {
        loop {
            if matches!(
                prompt_status.borrow().as_ref().map(|status| &status.phase),
                Some(PromptPhase::Completed(PromptCompletion::Local))
            ) {
                break;
            }
            prompt_status.changed().await.unwrap();
        }
    })
    .await
    .unwrap();

    runtime.shutdown().await.unwrap();
}

#[tokio::test]
async fn supervises_an_unexpected_child_exit() {
    let runtime = OmpRuntime::spawn(fixture_config()).await.unwrap();
    runtime.prompt("exit", None, None).await.unwrap();

    let status = time::timeout(Duration::from_secs(2), runtime.wait())
        .await
        .unwrap();
    assert!(matches!(
        status,
        RuntimeStatus::Exited(exit) if exit.code == Some(7) && !exit.success && !exit.forced
    ));
}
