//! Runtime configuration loaded from `.env` and `NIUMA_` environment variables.

use std::{
    collections::HashMap,
    env,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail};

#[derive(Debug, Clone)]
pub struct Settings {
    pub host: String,
    pub port: u16,
    pub log_level: String,
    pub database_url: String,
    pub database_pool_size: u32,
    pub database_connect_timeout: Duration,
    pub challenge_ttl_seconds: i64,
    pub pair_token_ttl_seconds: i64,
    pub session_token_ttl_seconds: i64,
    pub nonce_ttl_seconds: i64,
    pub auth_timestamp_tolerance_seconds: i64,
    pub pair_token_max_attempts: i32,
    pub transfer_storage_dir: PathBuf,
    pub transfer_ttl_seconds: i64,
    pub transfer_max_encrypted_bytes: usize,
    pub apns_key_id: Option<String>,
    pub apns_team_id: Option<String>,
    pub apns_topic: Option<String>,
    pub apns_auth_key_path: Option<PathBuf>,
    pub apns_auth_key_pem: Option<String>,
    pub apns_environment: String,
}

impl Settings {
    /// Load configuration using local `.env` values before process env values.
    pub fn load() -> Result<Self> {
        let env_file = resolve_env_file();
        let file_values = match env_file.as_deref() {
            Some(path) => read_env_file(path)?,
            None => HashMap::new(),
        };
        let database_url = get_string(&file_values, "NIUMA_DATABASE_URL", "")?;
        validate_database_url(&database_url)?;

        Ok(Self {
            host: get_string(&file_values, "NIUMA_HOST", "127.0.0.1")?,
            port: get_parse(&file_values, "NIUMA_PORT", 8000)?,
            log_level: get_string(&file_values, "NIUMA_LOG_LEVEL", "info")?,
            database_url,
            database_pool_size: get_parse(&file_values, "NIUMA_DATABASE_POOL_SIZE", 5)?,
            database_connect_timeout: Duration::from_secs_f64(get_parse(
                &file_values,
                "NIUMA_DATABASE_CONNECT_TIMEOUT_SECONDS",
                10.0,
            )?),
            challenge_ttl_seconds: get_parse(&file_values, "NIUMA_CHALLENGE_TTL_SECONDS", 120)?,
            pair_token_ttl_seconds: get_parse(&file_values, "NIUMA_PAIR_TOKEN_TTL_SECONDS", 300)?,
            session_token_ttl_seconds: get_parse(
                &file_values,
                "NIUMA_SESSION_TOKEN_TTL_SECONDS",
                3600,
            )?,
            nonce_ttl_seconds: get_parse(&file_values, "NIUMA_NONCE_TTL_SECONDS", 600)?,
            auth_timestamp_tolerance_seconds: get_parse(
                &file_values,
                "NIUMA_AUTH_TIMESTAMP_TOLERANCE_SECONDS",
                120,
            )?,
            pair_token_max_attempts: get_parse(&file_values, "NIUMA_PAIR_TOKEN_MAX_ATTEMPTS", 5)?,
            transfer_storage_dir: transfer_dir(&file_values)?,
            transfer_ttl_seconds: get_parse(&file_values, "NIUMA_TRANSFER_TTL_SECONDS", 1800)?,
            transfer_max_encrypted_bytes: get_parse(
                &file_values,
                "NIUMA_TRANSFER_MAX_ENCRYPTED_BYTES",
                100 * 1024 * 1024,
            )?,
            apns_key_id: get_optional_string(&file_values, "NIUMA_APNS_KEY_ID")?,
            apns_team_id: get_optional_string(&file_values, "NIUMA_APNS_TEAM_ID")?,
            apns_topic: get_optional_string(&file_values, "NIUMA_APNS_TOPIC")?,
            apns_auth_key_path: get_optional_string(&file_values, "NIUMA_APNS_AUTH_KEY_PATH")?
                .map(PathBuf::from),
            apns_auth_key_pem: get_optional_string(&file_values, "NIUMA_APNS_AUTH_KEY_PEM")?,
            apns_environment: get_string(&file_values, "NIUMA_APNS_ENVIRONMENT", "sandbox")?,
        })
    }
}

fn resolve_env_file() -> Option<PathBuf> {
    let cwd = env::current_dir().ok()?.join(".env");
    if cwd.exists() {
        return Some(cwd);
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".env");
    manifest.exists().then_some(manifest)
}

fn read_env_file(path: &Path) -> Result<HashMap<String, String>> {
    let mut values = HashMap::new();
    for item in dotenvy::from_path_iter(path)
        .with_context(|| format!("failed to read {}", path.display()))?
    {
        let (key, value) = item.with_context(|| format!("failed to parse {}", path.display()))?;
        values.insert(key, value);
    }
    Ok(values)
}

fn get_string(values: &HashMap<String, String>, key: &str, default: &str) -> Result<String> {
    Ok(values
        .get(key)
        .cloned()
        .or_else(|| env::var(key).ok())
        .unwrap_or_else(|| default.to_string()))
}

fn get_optional_string(values: &HashMap<String, String>, key: &str) -> Result<Option<String>> {
    Ok(values
        .get(key)
        .cloned()
        .or_else(|| env::var(key).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty()))
}

fn get_parse<T>(values: &HashMap<String, String>, key: &str, default: T) -> Result<T>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    let Some(raw) = values.get(key).cloned().or_else(|| env::var(key).ok()) else {
        return Ok(default);
    };
    raw.parse()
        .map_err(|error| anyhow::anyhow!("invalid {key}={raw}: {error}"))
}

fn transfer_dir(values: &HashMap<String, String>) -> Result<PathBuf> {
    let configured = get_string(values, "NIUMA_TRANSFER_STORAGE_DIR", "")?;
    if !configured.trim().is_empty() {
        return Ok(PathBuf::from(configured));
    }
    Ok(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".tmp/transfers"))
}

fn validate_database_url(database_url: &str) -> Result<()> {
    let trimmed = database_url.trim();
    if trimmed.is_empty() {
        bail!("NIUMA_DATABASE_URL must be configured");
    }
    if !trimmed.starts_with("postgresql://") && !trimmed.starts_with("postgres://") {
        bail!("NIUMA_DATABASE_URL must use PostgreSQL");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_database_url;

    #[test]
    fn database_url_requires_postgres_scheme() {
        assert!(validate_database_url("").is_err());
        assert!(validate_database_url("sqlite://local.db").is_err());
        assert!(validate_database_url("postgresql://localhost/db").is_ok());
        assert!(validate_database_url("postgres://localhost/db").is_ok());
    }
}
