use std::num::NonZeroU64;

use omp_control_protocol::{
    AgentId, AgentLifecycle, ClientPlatform, ControlRequest, ControlResponse, DeviceId,
    DeviceScopes, DeviceToken, EventSequence, OperationId, PairingSecret, ResponseOutcome, RunId,
    StateRevision,
};
use ompd::persistence::*;
use tempfile::TempDir;

fn database_path(directory: &TempDir) -> std::path::PathBuf {
    directory.path().join("ompd.sqlite3")
}

fn device_record() -> DeviceRecord {
    DeviceRecord {
        device_id: DeviceId::new("device-1").unwrap(),
        name: "Test Phone".into(),
        platform: ClientPlatform::Mobile,
        scopes: DeviceScopes::all(),
        created_at_ms: 100,
        last_seen_at_ms: None,
        revoked_at_ms: None,
    }
}

#[test]
fn server_identity_sessions_and_interrupted_agents_survive_restart() {
    let directory = TempDir::new().unwrap();
    let path = database_path(&directory);
    let store = Store::open_at(&path, 100).unwrap();
    let server_id = store.server_id().clone();
    let agent_id = AgentId::new("agent-1").unwrap();
    store
        .upsert_agent(&AgentRecord {
            agent_id: agent_id.clone(),
            lifecycle: AgentLifecycle::Running,
            process_id: Some(42),
            active_run_id: Some(RunId::new("run-1").unwrap()),
            created_at_ms: 100,
            updated_at_ms: 120,
        })
        .unwrap();
    let session = SessionResumeRecord {
        agent_id: agent_id.clone(),
        session_id: "session-1".into(),
        session_file: "/tmp/session.jsonl".into(),
        revision: StateRevision(7),
        event_sequence: EventSequence(11),
        updated_at_ms: 120,
    };
    store.upsert_session(&session).unwrap();
    drop(store);

    let reopened = Store::open_at(&path, 200).unwrap();
    assert_eq!(reopened.server_id(), &server_id);
    let recovered = reopened.agent(&agent_id).unwrap().unwrap();
    assert_eq!(recovered.lifecycle, AgentLifecycle::Interrupted);
    assert_eq!(recovered.process_id, None);
    assert_eq!(recovered.active_run_id, None);
    assert_eq!(recovered.updated_at_ms, 200);
    assert_eq!(reopened.session(&agent_id).unwrap(), Some(session));
}

#[test]
fn device_revocation_survives_restart() {
    let directory = TempDir::new().unwrap();
    let path = database_path(&directory);
    let store = Store::open_at(&path, 100).unwrap();
    let record = device_record();
    let raw_token = "device-secret-token";
    let token = DeviceToken::new(raw_token);
    assert!(!format!("{token:?}").contains(raw_token));
    store.insert_device(&record, &token).unwrap();

    let authenticated = store
        .authenticate_device(&record.device_id, &token, 150)
        .unwrap();
    assert_eq!(authenticated.last_seen_at_ms, Some(150));
    assert!(store.revoke_device(&record.device_id, 175).unwrap());
    drop(store);
    assert!(!contains_bytes(
        &std::fs::read(&path).unwrap(),
        raw_token.as_bytes()
    ));

    let reopened = Store::open_at(&path, 200).unwrap();
    assert!(matches!(
        reopened.authenticate_device(&record.device_id, &token, 210),
        Err(DeviceAuthenticationError::Revoked)
    ));
    assert_eq!(
        reopened
            .device(&record.device_id)
            .unwrap()
            .unwrap()
            .revoked_at_ms,
        Some(175)
    );
}

#[test]
fn pairing_secrets_are_single_use_expiring_and_never_stored_raw() {
    let directory = TempDir::new().unwrap();
    let path = database_path(&directory);
    let store = Store::open_at(&path, 100).unwrap();
    let grant = store
        .create_pairing(
            "Test Phone",
            DeviceScopes::all(),
            100,
            NonZeroU64::new(50).unwrap(),
        )
        .unwrap();
    let raw_secret = grant.secret.expose_secret().to_owned();
    assert!(!format!("{grant:?}").contains(&raw_secret));

    let consumed = store
        .consume_pairing(&grant.pairing_id, &grant.secret, 125)
        .unwrap();
    assert_eq!(consumed.consumed_at_ms, Some(125));
    assert!(matches!(
        store.consume_pairing(&grant.pairing_id, &grant.secret, 126),
        Err(PairingError::Consumed)
    ));

    let expired = store
        .create_pairing(
            "Expired",
            DeviceScopes::all(),
            200,
            NonZeroU64::new(10).unwrap(),
        )
        .unwrap();
    assert!(matches!(
        store.consume_pairing(&expired.pairing_id, &expired.secret, 210),
        Err(PairingError::Expired)
    ));
    assert!(matches!(
        store.consume_pairing(
            &expired.pairing_id,
            &PairingSecret::new("wrong-secret"),
            205
        ),
        Err(PairingError::InvalidCredential)
    ));
    drop(store);

    let bytes = std::fs::read(&path).unwrap();
    assert!(!contains_bytes(&bytes, raw_secret.as_bytes()));
    let connection = rusqlite::Connection::open(&path).unwrap();
    let (kind, length): (String, i64) = connection
        .query_row(
            "SELECT typeof(secret_hash), length(secret_hash) FROM pairing_secrets LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(kind, "blob");
    assert_eq!(length, 32);
}

#[test]
fn operation_claims_prevent_duplicate_prompts_across_retries_and_restart() {
    let directory = TempDir::new().unwrap();
    let path = database_path(&directory);
    let store = Store::open_at(&path, 100).unwrap();
    let device = device_record();
    store
        .insert_device(&device, &DeviceToken::new("token"))
        .unwrap();
    let request = ControlRequest::Prompt {
        agent_id: AgentId::new("agent-1").unwrap(),
        message: "hello".into(),
        images: Vec::new(),
        streaming_behavior: None,
    };
    let key = OperationKey {
        device_id: device.device_id.clone(),
        operation_id: OperationId::new("operation-1").unwrap(),
    };

    let mut prompt_executions = 0;
    if store.claim_operation(&key, &request, 110).unwrap() == OperationClaim::Execute {
        prompt_executions += 1;
    }
    assert_eq!(
        store.claim_operation(&key, &request, 111).unwrap(),
        OperationClaim::InProgress
    );
    let outcome = ResponseOutcome::Success(Box::new(ControlResponse::Accepted));
    store.complete_operation(&key, &outcome, 120).unwrap();
    assert_eq!(
        store.claim_operation(&key, &request, 121).unwrap(),
        OperationClaim::Completed(outcome.clone())
    );
    assert_eq!(prompt_executions, 1);

    let pending_key = OperationKey {
        device_id: device.device_id.clone(),
        operation_id: OperationId::new("operation-2").unwrap(),
    };
    assert_eq!(
        store.claim_operation(&pending_key, &request, 130).unwrap(),
        OperationClaim::Execute
    );
    drop(store);

    let reopened = Store::open_at(&path, 200).unwrap();
    assert_eq!(
        reopened.claim_operation(&key, &request, 201).unwrap(),
        OperationClaim::Completed(outcome)
    );
    assert_eq!(
        reopened
            .claim_operation(&pending_key, &request, 201)
            .unwrap(),
        OperationClaim::Indeterminate
    );
    assert_eq!(prompt_executions, 1);
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}
