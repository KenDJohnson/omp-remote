use std::{
    fmt,
    num::{NonZeroU32, NonZeroU64},
    path::Path,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use omp_control_protocol::{
    AgentId, AgentLifecycle, CborCodec, ClientPlatform, ControlRequest, DeviceId, DeviceScopes,
    DeviceToken, EventSequence, PairingId, PairingSecret, ResponseOutcome, RunId, ServerId,
    StateRevision,
};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use uuid::Uuid;

use super::{
    AgentRecord, DeviceRecord, OperationClaim, OperationKey, PairingGrant, PairingRecord,
    SessionResumeRecord,
};

const DATABASE_VERSION: i64 = 1;
const DATABASE_CBOR_LIMIT: u32 = 1_048_576;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS metadata (
    key TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS agents (
    agent_id TEXT PRIMARY KEY NOT NULL,
    lifecycle TEXT NOT NULL,
    failure_reason TEXT,
    process_id INTEGER,
    active_run_id TEXT,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS sessions (
    agent_id TEXT PRIMARY KEY NOT NULL REFERENCES agents(agent_id) ON DELETE CASCADE,
    session_id TEXT NOT NULL,
    session_file TEXT NOT NULL,
    revision INTEGER NOT NULL,
    event_sequence INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS devices (
    device_id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    platform TEXT NOT NULL,
    scope_observe INTEGER NOT NULL,
    scope_prompt INTEGER NOT NULL,
    scope_mutate_session INTEGER NOT NULL,
    scope_stop_agent INTEGER NOT NULL,
    scope_answer_ui INTEGER NOT NULL,
    scope_administer_devices INTEGER NOT NULL,
    token_hash BLOB NOT NULL,
    created_at_ms INTEGER NOT NULL,
    last_seen_at_ms INTEGER,
    revoked_at_ms INTEGER
);

CREATE TABLE IF NOT EXISTS pairing_secrets (
    pairing_id TEXT PRIMARY KEY NOT NULL,
    requested_name TEXT NOT NULL,
    scope_observe INTEGER NOT NULL,
    scope_prompt INTEGER NOT NULL,
    scope_mutate_session INTEGER NOT NULL,
    scope_stop_agent INTEGER NOT NULL,
    scope_answer_ui INTEGER NOT NULL,
    scope_administer_devices INTEGER NOT NULL,
    secret_hash BLOB NOT NULL,
    created_at_ms INTEGER NOT NULL,
    expires_at_ms INTEGER NOT NULL,
    consumed_at_ms INTEGER
);

CREATE TABLE IF NOT EXISTS operations (
    device_id TEXT NOT NULL REFERENCES devices(device_id) ON DELETE CASCADE,
    operation_id TEXT NOT NULL,
    request_hash BLOB NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('pending', 'completed', 'indeterminate')),
    outcome_cbor BLOB,
    created_at_ms INTEGER NOT NULL,
    completed_at_ms INTEGER,
    PRIMARY KEY (device_id, operation_id)
);

CREATE INDEX IF NOT EXISTS operations_created_idx
    ON operations(device_id, created_at_ms DESC);

PRAGMA user_version = 1;
"#;

#[derive(Clone, Debug)]
pub struct Store {
    connection: Arc<Mutex<Connection>>,
    server_id: ServerId,
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        Self::open_at(path, unix_time_ms()?)
    }

    pub fn open_at(path: impl AsRef<Path>, now_ms: u64) -> Result<Self, StoreError> {
        let connection = Connection::open(path)?;
        Self::initialize(connection, now_ms)
    }

    pub fn open_in_memory_at(now_ms: u64) -> Result<Self, StoreError> {
        Self::initialize(Connection::open_in_memory()?, now_ms)
    }

    fn initialize(mut connection: Connection, now_ms: u64) -> Result<Self, StoreError> {
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        migrate(&mut connection)?;
        recover_interrupted(&connection, now_ms)?;
        let server_id = load_or_create_server_id(&mut connection)?;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
            server_id,
        })
    }

    #[must_use]
    pub fn server_id(&self) -> &ServerId {
        &self.server_id
    }

    pub fn upsert_agent(&self, record: &AgentRecord) -> Result<(), StoreError> {
        let (lifecycle, failure_reason) = encode_lifecycle(&record.lifecycle);
        self.connection()?.execute(
            r#"
            INSERT INTO agents (
                agent_id, lifecycle, failure_reason, process_id, active_run_id,
                created_at_ms, updated_at_ms
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            ON CONFLICT(agent_id) DO UPDATE SET
                lifecycle = excluded.lifecycle,
                failure_reason = excluded.failure_reason,
                process_id = excluded.process_id,
                active_run_id = excluded.active_run_id,
                updated_at_ms = excluded.updated_at_ms
            "#,
            params![
                record.agent_id.as_str(),
                lifecycle,
                failure_reason,
                record.process_id.map(i64::from),
                record.active_run_id.as_ref().map(RunId::as_str),
                sql_u64(record.created_at_ms, "agent created_at_ms")?,
                sql_u64(record.updated_at_ms, "agent updated_at_ms")?,
            ],
        )?;
        Ok(())
    }

    pub fn agent(&self, agent_id: &AgentId) -> Result<Option<AgentRecord>, StoreError> {
        let raw = self
            .connection()?
            .query_row(
                r#"
                SELECT agent_id, lifecycle, failure_reason, process_id, active_run_id,
                       created_at_ms, updated_at_ms
                FROM agents WHERE agent_id = ?1
                "#,
                [agent_id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<i64>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, i64>(6)?,
                    ))
                },
            )
            .optional()?;
        raw.map(decode_agent).transpose()
    }

    pub fn upsert_session(&self, record: &SessionResumeRecord) -> Result<(), StoreError> {
        self.connection()?.execute(
            r#"
            INSERT INTO sessions (
                agent_id, session_id, session_file, revision, event_sequence, updated_at_ms
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            ON CONFLICT(agent_id) DO UPDATE SET
                session_id = excluded.session_id,
                session_file = excluded.session_file,
                revision = excluded.revision,
                event_sequence = excluded.event_sequence,
                updated_at_ms = excluded.updated_at_ms
            "#,
            params![
                record.agent_id.as_str(),
                record.session_id,
                record.session_file,
                sql_u64(record.revision.0, "session revision")?,
                sql_u64(record.event_sequence.0, "session event sequence")?,
                sql_u64(record.updated_at_ms, "session updated_at_ms")?,
            ],
        )?;
        Ok(())
    }

    pub fn session(&self, agent_id: &AgentId) -> Result<Option<SessionResumeRecord>, StoreError> {
        let raw = self
            .connection()?
            .query_row(
                r#"
                SELECT agent_id, session_id, session_file, revision, event_sequence, updated_at_ms
                FROM sessions WHERE agent_id = ?1
                "#,
                [agent_id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, i64>(5)?,
                    ))
                },
            )
            .optional()?;
        raw.map(decode_session).transpose()
    }

    pub fn insert_device(
        &self,
        record: &DeviceRecord,
        token: &DeviceToken,
    ) -> Result<(), StoreError> {
        let hash = hash_secret(token.expose_secret().as_bytes());
        self.connection()?.execute(
            r#"
            INSERT INTO devices (
                device_id, name, platform,
                scope_observe, scope_prompt, scope_mutate_session,
                scope_stop_agent, scope_answer_ui, scope_administer_devices,
                token_hash, created_at_ms, last_seen_at_ms, revoked_at_ms
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13
            )
            "#,
            params![
                record.device_id.as_str(),
                record.name,
                encode_platform(record.platform),
                record.scopes.observe,
                record.scopes.prompt,
                record.scopes.mutate_session,
                record.scopes.stop_agent,
                record.scopes.answer_ui,
                record.scopes.administer_devices,
                hash.as_slice(),
                sql_u64(record.created_at_ms, "device created_at_ms")?,
                record
                    .last_seen_at_ms
                    .map(|value| sql_u64(value, "device last_seen_at_ms"))
                    .transpose()?,
                record
                    .revoked_at_ms
                    .map(|value| sql_u64(value, "device revoked_at_ms"))
                    .transpose()?,
            ],
        )?;
        Ok(())
    }

    pub fn device(&self, device_id: &DeviceId) -> Result<Option<DeviceRecord>, StoreError> {
        let connection = self.connection()?;
        load_device(&connection, device_id)
    }

    pub fn authenticate_device(
        &self,
        device_id: &DeviceId,
        token: &DeviceToken,
        now_ms: u64,
    ) -> Result<DeviceRecord, DeviceAuthenticationError> {
        let connection = self.connection()?;
        let raw = connection
            .query_row(
                r#"
                SELECT token_hash, revoked_at_ms FROM devices WHERE device_id = ?1
                "#,
                [device_id.as_str()],
                |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Option<i64>>(1)?)),
            )
            .optional()?;
        let Some((stored_hash, revoked_at_ms)) = raw else {
            return Err(DeviceAuthenticationError::InvalidCredential);
        };
        let candidate_hash = hash_secret(token.expose_secret().as_bytes());
        if stored_hash.len() != candidate_hash.len()
            || !bool::from(stored_hash.as_slice().ct_eq(candidate_hash.as_slice()))
        {
            return Err(DeviceAuthenticationError::InvalidCredential);
        }
        if revoked_at_ms.is_some() {
            return Err(DeviceAuthenticationError::Revoked);
        }
        connection.execute(
            "UPDATE devices SET last_seen_at_ms = ?2 WHERE device_id = ?1",
            params![
                device_id.as_str(),
                sql_u64(now_ms, "device last_seen_at_ms")?
            ],
        )?;
        load_device(&connection, device_id)?.ok_or(DeviceAuthenticationError::InvalidCredential)
    }

    pub fn revoke_device(&self, device_id: &DeviceId, now_ms: u64) -> Result<bool, StoreError> {
        let changed = self.connection()?.execute(
            r#"
            UPDATE devices SET revoked_at_ms = ?2
            WHERE device_id = ?1 AND revoked_at_ms IS NULL
            "#,
            params![device_id.as_str(), sql_u64(now_ms, "device revoked_at_ms")?],
        )?;
        Ok(changed == 1)
    }

    pub fn create_pairing(
        &self,
        requested_name: impl Into<String>,
        scopes: DeviceScopes,
        now_ms: u64,
        ttl_ms: NonZeroU64,
    ) -> Result<PairingGrant, StoreError> {
        let expires_at_ms = now_ms
            .checked_add(ttl_ms.get())
            .ok_or(StoreError::IntegerOverflow("pairing expiry"))?;
        let mut secret_bytes = [0_u8; 32];
        getrandom::fill(&mut secret_bytes)
            .map_err(|error| StoreError::Random(error.to_string()))?;
        let secret = PairingSecret::new(URL_SAFE_NO_PAD.encode(secret_bytes));
        let secret_hash = hash_secret(secret.expose_secret().as_bytes());
        let pairing_id =
            PairingId::new(Uuid::new_v4().to_string()).expect("UUID pairing IDs are non-empty");
        self.connection()?.execute(
            r#"
            INSERT INTO pairing_secrets (
                pairing_id, requested_name,
                scope_observe, scope_prompt, scope_mutate_session,
                scope_stop_agent, scope_answer_ui, scope_administer_devices,
                secret_hash, created_at_ms, expires_at_ms, consumed_at_ms
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, NULL)
            "#,
            params![
                pairing_id.as_str(),
                requested_name.into(),
                scopes.observe,
                scopes.prompt,
                scopes.mutate_session,
                scopes.stop_agent,
                scopes.answer_ui,
                scopes.administer_devices,
                secret_hash.as_slice(),
                sql_u64(now_ms, "pairing created_at_ms")?,
                sql_u64(expires_at_ms, "pairing expires_at_ms")?,
            ],
        )?;
        Ok(PairingGrant {
            pairing_id,
            secret,
            expires_at_ms,
        })
    }

    pub fn consume_pairing(
        &self,
        pairing_id: &PairingId,
        secret: &PairingSecret,
        now_ms: u64,
    ) -> Result<PairingRecord, PairingError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let raw =
            load_pairing_raw(&transaction, pairing_id)?.ok_or(PairingError::InvalidCredential)?;
        let candidate_hash = hash_secret(secret.expose_secret().as_bytes());
        if raw.secret_hash.len() != candidate_hash.len()
            || !bool::from(raw.secret_hash.as_slice().ct_eq(candidate_hash.as_slice()))
        {
            return Err(PairingError::InvalidCredential);
        }
        if raw.consumed_at_ms.is_some() {
            return Err(PairingError::Consumed);
        }
        if raw.expires_at_ms <= now_ms {
            return Err(PairingError::Expired);
        }
        let changed = transaction.execute(
            r#"
            UPDATE pairing_secrets SET consumed_at_ms = ?2
            WHERE pairing_id = ?1 AND consumed_at_ms IS NULL
            "#,
            params![
                pairing_id.as_str(),
                sql_u64(now_ms, "pairing consumed_at_ms")?
            ],
        )?;
        if changed != 1 {
            return Err(PairingError::Consumed);
        }
        transaction.commit()?;
        Ok(PairingRecord {
            pairing_id: raw.pairing_id,
            requested_name: raw.requested_name,
            scopes: raw.scopes,
            created_at_ms: raw.created_at_ms,
            expires_at_ms: raw.expires_at_ms,
            consumed_at_ms: Some(now_ms),
        })
    }

    pub fn claim_operation(
        &self,
        key: &OperationKey,
        request: &ControlRequest,
        now_ms: u64,
    ) -> Result<OperationClaim, OperationError> {
        let codec = database_codec();
        let encoded = codec.encode(request)?;
        let request_hash = hash_secret(&encoded);
        let connection = self.connection()?;
        let inserted = connection.execute(
            r#"
            INSERT OR IGNORE INTO operations (
                device_id, operation_id, request_hash, status,
                outcome_cbor, created_at_ms, completed_at_ms
            ) VALUES (?1, ?2, ?3, 'pending', NULL, ?4, NULL)
            "#,
            params![
                key.device_id.as_str(),
                key.operation_id.as_str(),
                request_hash.as_slice(),
                sql_u64(now_ms, "operation created_at_ms")?,
            ],
        )?;
        if inserted == 1 {
            return Ok(OperationClaim::Execute);
        }

        let (stored_hash, status, outcome): (Vec<u8>, String, Option<Vec<u8>>) = connection
            .query_row(
                r#"
                SELECT request_hash, status, outcome_cbor
                FROM operations WHERE device_id = ?1 AND operation_id = ?2
                "#,
                params![key.device_id.as_str(), key.operation_id.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?;
        if stored_hash != request_hash {
            return Err(OperationError::ConflictingRequest);
        }
        match status.as_str() {
            "pending" => Ok(OperationClaim::InProgress),
            "indeterminate" => Ok(OperationClaim::Indeterminate),
            "completed" => {
                let bytes = outcome.ok_or_else(|| {
                    OperationError::Store(StoreError::CorruptData(
                        "completed operation has no outcome".into(),
                    ))
                })?;
                Ok(OperationClaim::Completed(codec.decode(&bytes)?))
            }
            other => Err(OperationError::Store(StoreError::CorruptData(format!(
                "unknown operation status {other}"
            )))),
        }
    }

    pub fn complete_operation(
        &self,
        key: &OperationKey,
        outcome: &ResponseOutcome,
        now_ms: u64,
    ) -> Result<(), OperationError> {
        let encoded = database_codec().encode(outcome)?;
        let changed = self.connection()?.execute(
            r#"
            UPDATE operations
            SET status = 'completed', outcome_cbor = ?3, completed_at_ms = ?4
            WHERE device_id = ?1 AND operation_id = ?2 AND status = 'pending'
            "#,
            params![
                key.device_id.as_str(),
                key.operation_id.as_str(),
                encoded,
                sql_u64(now_ms, "operation completed_at_ms")?,
            ],
        )?;
        if changed == 1 {
            Ok(())
        } else {
            Err(OperationError::NotPending)
        }
    }

    pub fn prune_operations(
        &self,
        device_id: &DeviceId,
        older_than_ms: u64,
        max_count: usize,
    ) -> Result<usize, StoreError> {
        let connection = self.connection()?;
        let old = connection.execute(
            "DELETE FROM operations WHERE device_id = ?1 AND created_at_ms < ?2",
            params![
                device_id.as_str(),
                sql_u64(older_than_ms, "operation prune cutoff")?
            ],
        )?;
        let excess = connection.execute(
            r#"
            DELETE FROM operations WHERE rowid IN (
                SELECT rowid FROM operations
                WHERE device_id = ?1
                ORDER BY created_at_ms DESC, operation_id DESC
                LIMIT -1 OFFSET ?2
            )
            "#,
            params![
                device_id.as_str(),
                i64::try_from(max_count)
                    .map_err(|_| StoreError::IntegerOverflow("operation max count"))?
            ],
        )?;
        Ok(old + excess)
    }

    fn connection(&self) -> Result<std::sync::MutexGuard<'_, Connection>, StoreError> {
        self.connection.lock().map_err(|_| StoreError::Poisoned)
    }
}

fn migrate(connection: &mut Connection) -> Result<(), StoreError> {
    let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version > DATABASE_VERSION {
        return Err(StoreError::UnsupportedDatabaseVersion(version));
    }
    if version == 0 {
        connection.execute_batch(SCHEMA)?;
    }
    Ok(())
}

fn recover_interrupted(connection: &Connection, now_ms: u64) -> Result<(), StoreError> {
    let now_ms = sql_u64(now_ms, "recovery timestamp")?;
    connection.execute(
        r#"
        UPDATE agents
        SET lifecycle = 'interrupted', failure_reason = NULL,
            process_id = NULL, active_run_id = NULL, updated_at_ms = ?1
        WHERE lifecycle IN ('starting', 'running', 'stopping')
        "#,
        [now_ms],
    )?;
    connection.execute(
        "UPDATE operations SET status = 'indeterminate' WHERE status = 'pending'",
        [],
    )?;
    Ok(())
}

fn load_or_create_server_id(connection: &mut Connection) -> Result<ServerId, StoreError> {
    if let Some(server_id) = connection
        .query_row(
            "SELECT value FROM metadata WHERE key = 'server_id'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?
    {
        return ServerId::new(server_id)
            .map_err(|error| StoreError::CorruptData(error.to_string()));
    }
    let server_id =
        ServerId::new(Uuid::new_v4().to_string()).expect("UUID server IDs are non-empty");
    connection.execute(
        "INSERT INTO metadata (key, value) VALUES ('server_id', ?1)",
        [server_id.as_str()],
    )?;
    Ok(server_id)
}

fn load_device(
    connection: &Connection,
    device_id: &DeviceId,
) -> Result<Option<DeviceRecord>, StoreError> {
    let raw = connection
        .query_row(
            r#"
            SELECT device_id, name, platform,
                   scope_observe, scope_prompt, scope_mutate_session,
                   scope_stop_agent, scope_answer_ui, scope_administer_devices,
                   created_at_ms, last_seen_at_ms, revoked_at_ms
            FROM devices WHERE device_id = ?1
            "#,
            [device_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, bool>(3)?,
                    row.get::<_, bool>(4)?,
                    row.get::<_, bool>(5)?,
                    row.get::<_, bool>(6)?,
                    row.get::<_, bool>(7)?,
                    row.get::<_, bool>(8)?,
                    row.get::<_, i64>(9)?,
                    row.get::<_, Option<i64>>(10)?,
                    row.get::<_, Option<i64>>(11)?,
                ))
            },
        )
        .optional()?;
    raw.map(
        |(
            device_id,
            name,
            platform,
            observe,
            prompt,
            mutate_session,
            stop_agent,
            answer_ui,
            administer_devices,
            created_at_ms,
            last_seen_at_ms,
            revoked_at_ms,
        )| {
            Ok(DeviceRecord {
                device_id: DeviceId::new(device_id)
                    .map_err(|error| StoreError::CorruptData(error.to_string()))?,
                name,
                platform: decode_platform(&platform)?,
                scopes: DeviceScopes {
                    observe,
                    prompt,
                    mutate_session,
                    stop_agent,
                    answer_ui,
                    administer_devices,
                },
                created_at_ms: wire_u64(created_at_ms, "device created_at_ms")?,
                last_seen_at_ms: last_seen_at_ms
                    .map(|value| wire_u64(value, "device last_seen_at_ms"))
                    .transpose()?,
                revoked_at_ms: revoked_at_ms
                    .map(|value| wire_u64(value, "device revoked_at_ms"))
                    .transpose()?,
            })
        },
    )
    .transpose()
}

#[derive(Debug)]
struct RawPairing {
    pairing_id: PairingId,
    requested_name: String,
    scopes: DeviceScopes,
    secret_hash: Vec<u8>,
    created_at_ms: u64,
    expires_at_ms: u64,
    consumed_at_ms: Option<u64>,
}

fn load_pairing_raw(
    transaction: &Transaction<'_>,
    pairing_id: &PairingId,
) -> Result<Option<RawPairing>, StoreError> {
    let raw = transaction
        .query_row(
            r#"
            SELECT pairing_id, requested_name,
                   scope_observe, scope_prompt, scope_mutate_session,
                   scope_stop_agent, scope_answer_ui, scope_administer_devices,
                   secret_hash, created_at_ms, expires_at_ms, consumed_at_ms
            FROM pairing_secrets WHERE pairing_id = ?1
            "#,
            [pairing_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, bool>(2)?,
                    row.get::<_, bool>(3)?,
                    row.get::<_, bool>(4)?,
                    row.get::<_, bool>(5)?,
                    row.get::<_, bool>(6)?,
                    row.get::<_, bool>(7)?,
                    row.get::<_, Vec<u8>>(8)?,
                    row.get::<_, i64>(9)?,
                    row.get::<_, i64>(10)?,
                    row.get::<_, Option<i64>>(11)?,
                ))
            },
        )
        .optional()?;
    raw.map(
        |(
            pairing_id,
            requested_name,
            observe,
            prompt,
            mutate_session,
            stop_agent,
            answer_ui,
            administer_devices,
            secret_hash,
            created_at_ms,
            expires_at_ms,
            consumed_at_ms,
        )| {
            Ok(RawPairing {
                pairing_id: PairingId::new(pairing_id)
                    .map_err(|error| StoreError::CorruptData(error.to_string()))?,
                requested_name,
                scopes: DeviceScopes {
                    observe,
                    prompt,
                    mutate_session,
                    stop_agent,
                    answer_ui,
                    administer_devices,
                },
                secret_hash,
                created_at_ms: wire_u64(created_at_ms, "pairing created_at_ms")?,
                expires_at_ms: wire_u64(expires_at_ms, "pairing expires_at_ms")?,
                consumed_at_ms: consumed_at_ms
                    .map(|value| wire_u64(value, "pairing consumed_at_ms"))
                    .transpose()?,
            })
        },
    )
    .transpose()
}

fn decode_agent(
    raw: (
        String,
        String,
        Option<String>,
        Option<i64>,
        Option<String>,
        i64,
        i64,
    ),
) -> Result<AgentRecord, StoreError> {
    let (
        agent_id,
        lifecycle,
        failure_reason,
        process_id,
        active_run_id,
        created_at_ms,
        updated_at_ms,
    ) = raw;
    Ok(AgentRecord {
        agent_id: AgentId::new(agent_id)
            .map_err(|error| StoreError::CorruptData(error.to_string()))?,
        lifecycle: decode_lifecycle(&lifecycle, failure_reason)?,
        process_id: process_id
            .map(|value| {
                u32::try_from(value)
                    .map_err(|_| StoreError::CorruptData("invalid process ID".into()))
            })
            .transpose()?,
        active_run_id: active_run_id
            .map(|value| {
                RunId::new(value).map_err(|error| StoreError::CorruptData(error.to_string()))
            })
            .transpose()?,
        created_at_ms: wire_u64(created_at_ms, "agent created_at_ms")?,
        updated_at_ms: wire_u64(updated_at_ms, "agent updated_at_ms")?,
    })
}

fn decode_session(
    raw: (String, String, String, i64, i64, i64),
) -> Result<SessionResumeRecord, StoreError> {
    Ok(SessionResumeRecord {
        agent_id: AgentId::new(raw.0)
            .map_err(|error| StoreError::CorruptData(error.to_string()))?,
        session_id: raw.1,
        session_file: raw.2,
        revision: StateRevision(wire_u64(raw.3, "session revision")?),
        event_sequence: EventSequence(wire_u64(raw.4, "session event sequence")?),
        updated_at_ms: wire_u64(raw.5, "session updated_at_ms")?,
    })
}

fn encode_lifecycle(lifecycle: &AgentLifecycle) -> (&'static str, Option<&str>) {
    match lifecycle {
        AgentLifecycle::Starting => ("starting", None),
        AgentLifecycle::Idle => ("idle", None),
        AgentLifecycle::Running => ("running", None),
        AgentLifecycle::Stopping => ("stopping", None),
        AgentLifecycle::Stopped => ("stopped", None),
        AgentLifecycle::Interrupted => ("interrupted", None),
        AgentLifecycle::Failed { reason } => ("failed", Some(reason)),
    }
}

fn decode_lifecycle(
    lifecycle: &str,
    failure_reason: Option<String>,
) -> Result<AgentLifecycle, StoreError> {
    match lifecycle {
        "starting" => Ok(AgentLifecycle::Starting),
        "idle" => Ok(AgentLifecycle::Idle),
        "running" => Ok(AgentLifecycle::Running),
        "stopping" => Ok(AgentLifecycle::Stopping),
        "stopped" => Ok(AgentLifecycle::Stopped),
        "interrupted" => Ok(AgentLifecycle::Interrupted),
        "failed" => Ok(AgentLifecycle::Failed {
            reason: failure_reason.ok_or_else(|| {
                StoreError::CorruptData("failed agent has no failure reason".into())
            })?,
        }),
        other => Err(StoreError::CorruptData(format!(
            "unknown agent lifecycle {other}"
        ))),
    }
}

fn encode_platform(platform: ClientPlatform) -> &'static str {
    match platform {
        ClientPlatform::Web => "web",
        ClientPlatform::Mobile => "mobile",
        ClientPlatform::Desktop => "desktop",
        ClientPlatform::Cli => "cli",
        ClientPlatform::Service => "service",
    }
}

fn decode_platform(platform: &str) -> Result<ClientPlatform, StoreError> {
    match platform {
        "web" => Ok(ClientPlatform::Web),
        "mobile" => Ok(ClientPlatform::Mobile),
        "desktop" => Ok(ClientPlatform::Desktop),
        "cli" => Ok(ClientPlatform::Cli),
        "service" => Ok(ClientPlatform::Service),
        other => Err(StoreError::CorruptData(format!(
            "unknown device platform {other}"
        ))),
    }
}

fn hash_secret(secret: &[u8]) -> [u8; 32] {
    Sha256::digest(secret).into()
}

fn database_codec() -> CborCodec {
    CborCodec::new(
        NonZeroU32::new(DATABASE_CBOR_LIMIT).expect("the database CBOR limit is non-zero"),
    )
}

fn sql_u64(value: u64, field: &'static str) -> Result<i64, StoreError> {
    i64::try_from(value).map_err(|_| StoreError::IntegerOverflow(field))
}

fn wire_u64(value: i64, field: &'static str) -> Result<u64, StoreError> {
    u64::try_from(value).map_err(|_| StoreError::CorruptData(format!("negative {field}")))
}

fn unix_time_ms() -> Result<u64, StoreError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| StoreError::Clock(error.to_string()))?;
    u64::try_from(duration.as_millis()).map_err(|_| StoreError::IntegerOverflow("system time"))
}

#[derive(Debug)]
pub enum StoreError {
    Database(rusqlite::Error),
    Codec(omp_control_protocol::CborCodecError),
    UnsupportedDatabaseVersion(i64),
    IntegerOverflow(&'static str),
    CorruptData(String),
    Random(String),
    Clock(String),
    Poisoned,
}

impl fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(error) => write!(formatter, "SQLite error: {error}"),
            Self::Codec(error) => error.fmt(formatter),
            Self::UnsupportedDatabaseVersion(version) => {
                write!(
                    formatter,
                    "database version {version} is newer than supported"
                )
            }
            Self::IntegerOverflow(field) => write!(formatter, "{field} exceeds SQLite range"),
            Self::CorruptData(message) => write!(formatter, "corrupt database data: {message}"),
            Self::Random(message) => {
                write!(formatter, "secure random generation failed: {message}")
            }
            Self::Clock(message) => write!(formatter, "system clock failed: {message}"),
            Self::Poisoned => formatter.write_str("SQLite connection lock was poisoned"),
        }
    }
}

impl std::error::Error for StoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Database(error) => Some(error),
            Self::Codec(error) => Some(error),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for StoreError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error)
    }
}

impl From<omp_control_protocol::CborCodecError> for StoreError {
    fn from(error: omp_control_protocol::CborCodecError) -> Self {
        Self::Codec(error)
    }
}

#[derive(Debug)]
pub enum DeviceAuthenticationError {
    InvalidCredential,
    Revoked,
    Store(StoreError),
}

impl fmt::Display for DeviceAuthenticationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCredential => formatter.write_str("invalid device credential"),
            Self::Revoked => formatter.write_str("device credential was revoked"),
            Self::Store(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for DeviceAuthenticationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Store(error) => Some(error),
            _ => None,
        }
    }
}

impl From<StoreError> for DeviceAuthenticationError {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

impl From<rusqlite::Error> for DeviceAuthenticationError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Store(StoreError::Database(error))
    }
}

#[derive(Debug)]
pub enum PairingError {
    InvalidCredential,
    Expired,
    Consumed,
    Store(StoreError),
}

impl fmt::Display for PairingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCredential => formatter.write_str("invalid pairing credential"),
            Self::Expired => formatter.write_str("pairing credential expired"),
            Self::Consumed => formatter.write_str("pairing credential was already consumed"),
            Self::Store(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for PairingError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Store(error) => Some(error),
            _ => None,
        }
    }
}

impl From<StoreError> for PairingError {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

impl From<rusqlite::Error> for PairingError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Store(StoreError::Database(error))
    }
}

#[derive(Debug)]
pub enum OperationError {
    ConflictingRequest,
    NotPending,
    Store(StoreError),
}

impl fmt::Display for OperationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConflictingRequest => {
                formatter.write_str("operation ID was reused for a different request")
            }
            Self::NotPending => formatter.write_str("operation is not pending"),
            Self::Store(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for OperationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Store(error) => Some(error),
            _ => None,
        }
    }
}

impl From<StoreError> for OperationError {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

impl From<rusqlite::Error> for OperationError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Store(StoreError::Database(error))
    }
}

impl From<omp_control_protocol::CborCodecError> for OperationError {
    fn from(error: omp_control_protocol::CborCodecError) -> Self {
        Self::Store(StoreError::Codec(error))
    }
}
