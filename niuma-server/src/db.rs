//! PostgreSQL access for Niuma control-plane state.

use chrono::{DateTime, Duration, Utc};
use sqlx::{PgPool, Row};

use crate::{
    config::Settings,
    crypto,
    error::ApiError,
    models::{DeviceRegisterRequest, PairConfirmRequest, VerifyRequest},
};

#[derive(Debug, Clone)]
pub struct PairTokenRecord {
    pub pair_token: String,
    pub agent_id: String,
    pub agent_pairing_public_key: String,
    pub expires_at: DateTime<Utc>,
    pub failed_attempts: i32,
    pub consumed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct TransferBinding {
    pub source_device_id: String,
    pub target_device_id: String,
    pub direction: String,
}

/// Create or repair the small PostgreSQL schema owned by the Rust server.
pub async fn init_schema(pool: &PgPool) -> Result<(), sqlx::Error> {
    if schema_conflicts(pool).await? {
        drop_control_plane_tables(pool).await?;
    }
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS ios_devices (
            ios_device_id TEXT PRIMARY KEY,
            device_name TEXT NOT NULL,
            public_key TEXT NOT NULL,
            push_token TEXT,
            last_seen_at TIMESTAMPTZ NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await?;
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS agents (
            agent_id TEXT PRIMARY KEY,
            device_name TEXT NOT NULL,
            public_key TEXT NOT NULL,
            os_type TEXT,
            last_seen_at TIMESTAMPTZ NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await?;
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS auth_challenges (
            challenge_id TEXT PRIMARY KEY,
            device_id TEXT NOT NULL,
            challenge TEXT NOT NULL,
            expires_at TIMESTAMPTZ NOT NULL,
            status TEXT NOT NULL,
            used_at TIMESTAMPTZ
        )
        "#,
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_auth_challenges_device_id ON auth_challenges(device_id)",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_auth_challenges_expires_at ON auth_challenges(expires_at)",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS session_tokens (
            token TEXT PRIMARY KEY,
            device_id TEXT NOT NULL,
            expires_at TIMESTAMPTZ NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_session_tokens_device_id ON session_tokens(device_id)",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_session_tokens_expires_at ON session_tokens(expires_at)",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS used_nonces (
            nonce TEXT PRIMARY KEY,
            expires_at TIMESTAMPTZ NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await?;
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS pair_tokens (
            pair_token TEXT PRIMARY KEY,
            agent_id TEXT NOT NULL,
            agent_pairing_public_key TEXT NOT NULL,
            expires_at TIMESTAMPTZ NOT NULL,
            failed_attempts INTEGER NOT NULL DEFAULT 0,
            consumed_at TIMESTAMPTZ,
            created_at TIMESTAMPTZ NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_pair_tokens_agent_id ON pair_tokens(agent_id)")
        .execute(pool)
        .await?;
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS pair_bindings (
            binding_id TEXT PRIMARY KEY,
            ios_device_id TEXT NOT NULL,
            agent_id TEXT NOT NULL,
            status TEXT NOT NULL,
            created_at TIMESTAMPTZ NOT NULL,
            revoked_at TIMESTAMPTZ
        )
        "#,
    )
    .execute(pool)
    .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_pair_bindings_ios ON pair_bindings(ios_device_id)")
        .execute(pool)
        .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_pair_bindings_agent ON pair_bindings(agent_id)")
        .execute(pool)
        .await?;
    Ok(())
}

async fn schema_conflicts(pool: &PgPool) -> Result<bool, sqlx::Error> {
    let agents_online = column_exists(pool, "agents", "online_status").await?;
    let missing_pairing_key = table_exists(pool, "pair_tokens").await?
        && !column_exists(pool, "pair_tokens", "agent_pairing_public_key").await?;
    let missing_revoked_at = table_exists(pool, "pair_bindings").await?
        && !column_exists(pool, "pair_bindings", "revoked_at").await?;
    Ok(agents_online || missing_pairing_key || missing_revoked_at)
}

async fn table_exists(pool: &PgPool, table: &str) -> Result<bool, sqlx::Error> {
    let row = sqlx::query(
        "SELECT EXISTS (SELECT 1 FROM information_schema.tables WHERE table_schema='public' AND table_name=$1)",
    )
    .bind(table)
    .fetch_one(pool)
    .await?;
    Ok(row.get::<bool, _>(0))
}

async fn column_exists(pool: &PgPool, table: &str, column: &str) -> Result<bool, sqlx::Error> {
    let row = sqlx::query(
        "SELECT EXISTS (SELECT 1 FROM information_schema.columns WHERE table_schema='public' AND table_name=$1 AND column_name=$2)",
    )
    .bind(table)
    .bind(column)
    .fetch_one(pool)
    .await?;
    Ok(row.get::<bool, _>(0))
}

async fn drop_control_plane_tables(pool: &PgPool) -> Result<(), sqlx::Error> {
    for table in [
        "pair_bindings",
        "pair_tokens",
        "used_nonces",
        "session_tokens",
        "auth_challenges",
        "agents",
        "ios_devices",
    ] {
        sqlx::query(&format!("DROP TABLE IF EXISTS {table} CASCADE"))
            .execute(pool)
            .await?;
    }
    Ok(())
}

pub async fn cleanup_expired(pool: &PgPool) -> Result<(), sqlx::Error> {
    let now = Utc::now();
    sqlx::query("DELETE FROM auth_challenges WHERE expires_at < $1")
        .bind(now)
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM session_tokens WHERE expires_at < $1")
        .bind(now)
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM used_nonces WHERE expires_at < $1")
        .bind(now)
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM pair_tokens WHERE expires_at < $1 OR consumed_at IS NOT NULL")
        .bind(now)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn register_device(
    pool: &PgPool,
    payload: &DeviceRegisterRequest,
) -> Result<(), ApiError> {
    let now = Utc::now();
    match payload.device_type.as_str() {
        "ios" => {
            sqlx::query(
                r#"
                INSERT INTO ios_devices (ios_device_id, device_name, public_key, push_token, last_seen_at)
                VALUES ($1, $2, $3, $4, $5)
                ON CONFLICT (ios_device_id) DO UPDATE SET
                    device_name = EXCLUDED.device_name,
                    public_key = EXCLUDED.public_key,
                    push_token = EXCLUDED.push_token,
                    last_seen_at = EXCLUDED.last_seen_at
                "#,
            )
            .bind(&payload.device_id)
            .bind(&payload.device_name)
            .bind(&payload.public_key)
            .bind(&payload.push_token)
            .bind(now)
            .execute(pool)
            .await?;
        }
        "agent" => {
            sqlx::query(
                r#"
                INSERT INTO agents (agent_id, device_name, public_key, os_type, last_seen_at)
                VALUES ($1, $2, $3, $4, $5)
                ON CONFLICT (agent_id) DO UPDATE SET
                    device_name = EXCLUDED.device_name,
                    public_key = EXCLUDED.public_key,
                    os_type = EXCLUDED.os_type,
                    last_seen_at = EXCLUDED.last_seen_at
                "#,
            )
            .bind(&payload.device_id)
            .bind(&payload.device_name)
            .bind(&payload.public_key)
            .bind(&payload.os_type)
            .bind(now)
            .execute(pool)
            .await?;
        }
        _ => {
            return Err(ApiError::BadRequest(
                "device_type must be ios or agent".to_string(),
            ));
        }
    }
    Ok(())
}

pub async fn device_exists(pool: &PgPool, device_id: &str) -> Result<bool, sqlx::Error> {
    Ok(public_key(pool, device_id).await?.is_some())
}

pub async fn public_key(pool: &PgPool, device_id: &str) -> Result<Option<String>, sqlx::Error> {
    if let Some(row) = sqlx::query("SELECT public_key FROM ios_devices WHERE ios_device_id=$1")
        .bind(device_id)
        .fetch_optional(pool)
        .await?
    {
        return Ok(Some(row.get("public_key")));
    }
    if let Some(row) = sqlx::query("SELECT public_key FROM agents WHERE agent_id=$1")
        .bind(device_id)
        .fetch_optional(pool)
        .await?
    {
        return Ok(Some(row.get("public_key")));
    }
    Ok(None)
}

pub async fn touch_device(pool: &PgPool, device_id: &str) -> Result<(), sqlx::Error> {
    let now = Utc::now();
    let ios = sqlx::query("UPDATE ios_devices SET last_seen_at=$1 WHERE ios_device_id=$2")
        .bind(now)
        .bind(device_id)
        .execute(pool)
        .await?;
    if ios.rows_affected() == 0 {
        sqlx::query("UPDATE agents SET last_seen_at=$1 WHERE agent_id=$2")
            .bind(now)
            .bind(device_id)
            .execute(pool)
            .await?;
    }
    Ok(())
}

pub async fn update_ios_push_token(
    pool: &PgPool,
    device_id: &str,
    push_token: &str,
) -> Result<bool, sqlx::Error> {
    let result =
        sqlx::query("UPDATE ios_devices SET push_token=$1, last_seen_at=$2 WHERE ios_device_id=$3")
            .bind(push_token)
            .bind(Utc::now())
            .bind(device_id)
            .execute(pool)
            .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn ios_push_token(pool: &PgPool, device_id: &str) -> Result<Option<String>, sqlx::Error> {
    let Some(row) = sqlx::query("SELECT push_token FROM ios_devices WHERE ios_device_id=$1")
        .bind(device_id)
        .fetch_optional(pool)
        .await?
    else {
        return Ok(None);
    };
    Ok(row.get::<Option<String>, _>("push_token"))
}

pub async fn issue_challenge(
    pool: &PgPool,
    settings: &Settings,
    device_id: &str,
) -> Result<(String, String, i64), ApiError> {
    if !device_exists(pool, device_id).await? {
        return Err(ApiError::NotFound(format!("unknown device_id={device_id}")));
    }
    let now = Utc::now();
    let challenge_id = crypto::random_token(16);
    let challenge = crypto::random_token(24);
    let expires_at = now + Duration::seconds(settings.challenge_ttl_seconds);
    sqlx::query(
        "INSERT INTO auth_challenges (challenge_id, device_id, challenge, expires_at, status) VALUES ($1, $2, $3, $4, 'issued')",
    )
    .bind(&challenge_id)
    .bind(device_id)
    .bind(&challenge)
    .bind(expires_at)
    .execute(pool)
    .await?;
    Ok((challenge_id, challenge, expires_at.timestamp()))
}

pub async fn verify_challenge(
    pool: &PgPool,
    settings: &Settings,
    payload: &VerifyRequest,
) -> Result<String, ApiError> {
    let now = Utc::now();
    let row = sqlx::query(
        "SELECT device_id, challenge, expires_at, status, used_at FROM auth_challenges WHERE challenge_id=$1",
    )
    .bind(&payload.challenge_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| ApiError::Unauthorized("challenge not found".to_string()))?;
    let device_id: String = row.get("device_id");
    let challenge: String = row.get("challenge");
    let expires_at: DateTime<Utc> = row.get("expires_at");
    let status: String = row.get("status");
    let used_at: Option<DateTime<Utc>> = row.get("used_at");
    if device_id != payload.device_id {
        return Err(ApiError::Unauthorized("challenge not found".to_string()));
    }
    if status != "issued" || used_at.is_some() {
        return Err(ApiError::Unauthorized(
            "challenge already consumed".to_string(),
        ));
    }
    if expires_at < now {
        set_challenge_status(pool, &payload.challenge_id, "expired", None).await?;
        return Err(ApiError::Unauthorized("challenge expired".to_string()));
    }
    if (now.timestamp() - payload.timestamp).abs() > settings.auth_timestamp_tolerance_seconds {
        set_challenge_status(pool, &payload.challenge_id, "rejected", None).await?;
        return Err(ApiError::Unauthorized("timestamp out of range".to_string()));
    }
    let nonce_used = sqlx::query("SELECT nonce FROM used_nonces WHERE nonce=$1")
        .bind(&payload.nonce)
        .fetch_optional(pool)
        .await?
        .is_some();
    if nonce_used {
        set_challenge_status(pool, &payload.challenge_id, "rejected", None).await?;
        return Err(ApiError::Unauthorized("nonce reused".to_string()));
    }
    let expected = crypto::auth_digest(
        &payload.device_id,
        &payload.challenge_id,
        &challenge,
        payload.timestamp,
        &payload.nonce,
    );
    if expected != payload.request_digest {
        set_challenge_status(pool, &payload.challenge_id, "rejected", None).await?;
        return Err(ApiError::Unauthorized(
            "request digest mismatch".to_string(),
        ));
    }
    let public_key = public_key(pool, &payload.device_id)
        .await?
        .ok_or_else(|| ApiError::Unauthorized("unknown device_id".to_string()))?;
    if !crypto::verify_ed25519(&public_key, &payload.request_digest, &payload.signature) {
        set_challenge_status(pool, &payload.challenge_id, "rejected", None).await?;
        return Err(ApiError::Unauthorized("signature invalid".to_string()));
    }
    let token = crypto::random_token(24);
    let token_expires_at = now + Duration::seconds(settings.session_token_ttl_seconds);
    set_challenge_status(pool, &payload.challenge_id, "verified", Some(now)).await?;
    sqlx::query("INSERT INTO used_nonces (nonce, expires_at) VALUES ($1, $2) ON CONFLICT (nonce) DO NOTHING")
        .bind(&payload.nonce)
        .bind(now + Duration::seconds(settings.nonce_ttl_seconds))
        .execute(pool)
        .await?;
    sqlx::query("INSERT INTO session_tokens (token, device_id, expires_at) VALUES ($1, $2, $3)")
        .bind(&token)
        .bind(&payload.device_id)
        .bind(token_expires_at)
        .execute(pool)
        .await?;
    Ok(token)
}

async fn set_challenge_status(
    pool: &PgPool,
    challenge_id: &str,
    status: &str,
    used_at: Option<DateTime<Utc>>,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE auth_challenges SET status=$1, used_at=$2 WHERE challenge_id=$3")
        .bind(status)
        .bind(used_at)
        .bind(challenge_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn validate_session_token(
    pool: &PgPool,
    token: &str,
    device_id: &str,
) -> Result<bool, sqlx::Error> {
    let Some(row) = sqlx::query("SELECT device_id, expires_at FROM session_tokens WHERE token=$1")
        .bind(token)
        .fetch_optional(pool)
        .await?
    else {
        return Ok(false);
    };
    let token_device_id: String = row.get("device_id");
    let expires_at: DateTime<Utc> = row.get("expires_at");
    Ok(token_device_id == device_id && expires_at >= Utc::now())
}

pub async fn require_session(
    pool: &PgPool,
    token: Option<&str>,
    device_id: &str,
) -> Result<(), ApiError> {
    let token =
        token.ok_or_else(|| ApiError::Unauthorized("missing X-Session-Token".to_string()))?;
    if validate_session_token(pool, token, device_id).await? {
        Ok(())
    } else {
        Err(ApiError::Unauthorized("invalid session token".to_string()))
    }
}

pub async fn issue_pair_token(
    pool: &PgPool,
    settings: &Settings,
    agent_id: &str,
    agent_pairing_public_key: &str,
) -> Result<(String, i64), ApiError> {
    let agent_exists = sqlx::query("SELECT agent_id FROM agents WHERE agent_id=$1")
        .bind(agent_id)
        .fetch_optional(pool)
        .await?
        .is_some();
    if !agent_exists {
        return Err(ApiError::NotFound("unknown agent".to_string()));
    }
    let now = Utc::now();
    let pair_token = crypto::random_token(18);
    let expires_at = now + Duration::seconds(settings.pair_token_ttl_seconds);
    sqlx::query(
        r#"
        INSERT INTO pair_tokens (pair_token, agent_id, agent_pairing_public_key, expires_at, failed_attempts, created_at)
        VALUES ($1, $2, $3, $4, 0, $5)
        "#,
    )
    .bind(&pair_token)
    .bind(agent_id)
    .bind(agent_pairing_public_key)
    .bind(expires_at)
    .bind(now)
    .execute(pool)
    .await?;
    Ok((pair_token, expires_at.timestamp()))
}

pub async fn pair_token_record(
    pool: &PgPool,
    pair_token: &str,
) -> Result<Option<PairTokenRecord>, sqlx::Error> {
    let Some(row) = sqlx::query(
        "SELECT pair_token, agent_id, agent_pairing_public_key, expires_at, failed_attempts, consumed_at FROM pair_tokens WHERE pair_token=$1",
    )
    .bind(pair_token)
    .fetch_optional(pool)
    .await?
    else {
        return Ok(None);
    };
    Ok(Some(PairTokenRecord {
        pair_token: row.get("pair_token"),
        agent_id: row.get("agent_id"),
        agent_pairing_public_key: row.get("agent_pairing_public_key"),
        expires_at: row.get("expires_at"),
        failed_attempts: row.get("failed_attempts"),
        consumed_at: row.get("consumed_at"),
    }))
}

pub async fn validate_pair_confirm(
    pool: &PgPool,
    settings: &Settings,
    payload: &PairConfirmRequest,
) -> Result<String, ApiError> {
    let token = pair_token_record(pool, &payload.pair_token)
        .await?
        .ok_or_else(|| ApiError::BadRequest("pair token invalid".to_string()))?;
    if token.pair_token != payload.pair_token || token.agent_id != payload.agent_id {
        return Err(ApiError::BadRequest("pair token invalid".to_string()));
    }
    if token.agent_pairing_public_key != payload.agent_pairing_public_key {
        return Err(ApiError::BadRequest(
            "agent pairing public key mismatch".to_string(),
        ));
    }
    if token.consumed_at.is_some() || token.expires_at < Utc::now() {
        return Err(ApiError::BadRequest("pair token expired".to_string()));
    }
    if token.failed_attempts >= settings.pair_token_max_attempts {
        return Err(ApiError::BadRequest("pair token locked".to_string()));
    }
    let digest = crypto::pair_digest(
        &payload.device_id,
        &payload.agent_id,
        &payload.pair_token,
        &payload.agent_pairing_public_key,
        &payload.encrypted_handshake,
    );
    if !crypto::verify_ed25519(&payload.public_key, &digest, &payload.signature) {
        record_pair_failure(pool, &payload.pair_token, settings).await?;
        return Err(ApiError::BadRequest("signature invalid".to_string()));
    }
    Ok(crypto::random_token(12))
}

pub async fn record_pair_failure(
    pool: &PgPool,
    pair_token: &str,
    settings: &Settings,
) -> Result<(), sqlx::Error> {
    let failed_attempts = pair_token_record(pool, pair_token)
        .await?
        .map(|record| record.failed_attempts + 1)
        .unwrap_or(1);
    let expires_at = if failed_attempts >= settings.pair_token_max_attempts {
        Some(Utc::now())
    } else {
        None
    };
    sqlx::query(
        "UPDATE pair_tokens SET failed_attempts=$1, expires_at=COALESCE($2, expires_at) WHERE pair_token=$3",
    )
    .bind(failed_attempts)
    .bind(expires_at)
    .bind(pair_token)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn commit_pair_confirm(
    pool: &PgPool,
    payload: &PairConfirmRequest,
    binding_id: &str,
) -> Result<(), ApiError> {
    let now = Utc::now();
    let register_payload = DeviceRegisterRequest {
        device_type: "ios".to_string(),
        device_id: payload.device_id.clone(),
        device_name: payload.device_name.clone(),
        public_key: payload.public_key.clone(),
        os_type: payload.os_type.clone(),
        push_token: payload.push_token.clone(),
    };
    register_device(pool, &register_payload).await?;
    sqlx::query(
        "UPDATE pair_bindings SET status='revoked', revoked_at=$1 WHERE ios_device_id=$2 AND agent_id=$3 AND status='active'",
    )
    .bind(now)
    .bind(&payload.device_id)
    .bind(&payload.agent_id)
    .execute(pool)
    .await?;
    sqlx::query(
        r#"
        INSERT INTO pair_bindings (binding_id, ios_device_id, agent_id, status, created_at)
        VALUES ($1, $2, $3, 'active', $4)
        "#,
    )
    .bind(binding_id)
    .bind(&payload.device_id)
    .bind(&payload.agent_id)
    .bind(now)
    .execute(pool)
    .await?;
    sqlx::query("UPDATE pair_tokens SET consumed_at=$1 WHERE pair_token=$2")
        .bind(now)
        .bind(&payload.pair_token)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn revoke_pairing(
    pool: &PgPool,
    device_id: &str,
    agent_id: &str,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE pair_bindings SET status='revoked', revoked_at=$1 WHERE ios_device_id=$2 AND agent_id=$3 AND status='active'",
    )
    .bind(Utc::now())
    .bind(device_id)
    .bind(agent_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn revoke_agent_pair_binding(
    pool: &PgPool,
    binding_id: &str,
    agent_id: &str,
) -> Result<Option<bool>, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE pair_bindings SET status='revoked', revoked_at=$1 WHERE binding_id=$2 AND agent_id=$3 AND status='active'",
    )
    .bind(Utc::now())
    .bind(binding_id)
    .bind(agent_id)
    .execute(pool)
    .await?;
    if result.rows_affected() > 0 {
        return Ok(Some(true));
    }

    let exists = sqlx::query(
        "SELECT binding_id FROM pair_bindings WHERE binding_id=$1 AND agent_id=$2 LIMIT 1",
    )
    .bind(binding_id)
    .bind(agent_id)
    .fetch_optional(pool)
    .await?
    .is_some();
    Ok(exists.then_some(false))
}

pub async fn is_paired(
    pool: &PgPool,
    device_id: &str,
    agent_id: &str,
) -> Result<bool, sqlx::Error> {
    let row = sqlx::query(
        "SELECT binding_id FROM pair_bindings WHERE ios_device_id=$1 AND agent_id=$2 AND status='active' LIMIT 1",
    )
    .bind(device_id)
    .bind(agent_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.is_some())
}

pub async fn require_transfer_binding(
    pool: &PgPool,
    binding: &TransferBinding,
) -> Result<(), ApiError> {
    let paired = match binding.direction.as_str() {
        "ios_to_agent" => {
            is_paired(pool, &binding.source_device_id, &binding.target_device_id).await?
        }
        "agent_to_ios" => {
            is_paired(pool, &binding.target_device_id, &binding.source_device_id).await?
        }
        _ => false,
    };
    if paired {
        Ok(())
    } else {
        Err(ApiError::Forbidden("devices are not paired".to_string()))
    }
}
