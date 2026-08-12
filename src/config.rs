use std::net::IpAddr;
use std::path::PathBuf;
use std::str::FromStr;

use serde::{Deserialize, Deserializer};

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    pub app: AppInfoConfig,
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub logging: LoggingConfig,
    pub files: FilesConfig,
    pub telegram: TelegramConfig,
    pub notification_worker: NotificationWorkerConfig,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppInfoConfig {
    pub name: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    #[serde(deserialize_with = "deserialize_loopback_ip")]
    pub host: IpAddr,
    pub port: u16,
    pub request_timeout_ms: u64,
}

fn deserialize_loopback_ip<'de, D>(deserializer: D) -> Result<IpAddr, D::Error>
where
    D: Deserializer<'de>,
{
    let host = IpAddr::deserialize(deserializer)?;
    if host.is_loopback() {
        Ok(host)
    } else {
        Err(serde::de::Error::custom(
            "server.host must be a loopback IP address",
        ))
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatabaseConfig {
    pub url: String,
    pub max_connections: u32,
    pub acquire_timeout_ms: u64,
    pub statement_timeout_ms: u64,
    pub run_migrations: bool,
    pub run_bootstrap: bool,
    pub bootstrap_file: PathBuf,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoggingConfig {
    pub level: String,
    pub format: LoggingFormat,
    pub file_enabled: bool,
    pub file_dir: String,
    pub file_prefix: String,
    pub file_suffix: String,
    pub rotation: LogRotation,
    pub also_stdout: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilesConfig {
    pub max_file_size_bytes: u64,
    pub max_files_per_assignment: usize,
    pub max_files_per_submission: usize,
    pub transfer_timeout_ms: u64,
    pub max_concurrent_transfers: usize,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TelegramConfig {
    pub enabled: bool,
    pub api_base_url: String,
    pub bot_token: Option<String>,
    pub request_timeout_ms: u64,
    pub polling_timeout_ms: u64,
    pub rate_limit_burst: u32,
    pub rate_limit_replenish_ms: u64,
    pub max_concurrent_updates: usize,
    pub shutdown_grace_ms: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NotificationWorkerConfig {
    pub enabled: bool,
    pub poll_interval_ms: u64,
    pub lease_duration_ms: u64,
    pub max_attempts: u32,
    pub retry_initial_ms: u64,
    pub retry_max_ms: u64,
    pub shutdown_grace_ms: u64,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoggingFormat {
    Compact,
    Pretty,
    Json,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogRotation {
    Daily,
    Hourly,
    Never,
}

impl AppConfig {
    pub fn load() -> Result<Self, ::config::ConfigError> {
        ::config::Config::builder()
            .add_source(::config::File::with_name("config/default"))
            .add_source(::config::File::with_name("config/local").required(false))
            .add_source(
                ::config::Environment::with_prefix("APP")
                    .prefix_separator("__")
                    .separator("__"),
            )
            .build()?
            .try_deserialize()
    }

    pub fn validate(self) -> Result<ValidatedAppConfig, ConfigValidationError> {
        validate_non_empty("app.name", &self.app.name)?;
        validate(self.server.port != 0, "server.port must be positive")?;
        validate_positive("server.request_timeout_ms", self.server.request_timeout_ms)?;

        sqlx::postgres::PgConnectOptions::from_str(&self.database.url)
            .map_err(|_| invalid("database.url must be a valid PostgreSQL URL"))?;
        validate(
            self.database.max_connections > 0,
            "database.max_connections must be positive",
        )?;
        validate_positive(
            "database.acquire_timeout_ms",
            self.database.acquire_timeout_ms,
        )?;
        validate_positive(
            "database.statement_timeout_ms",
            self.database.statement_timeout_ms,
        )?;

        tracing_subscriber::EnvFilter::try_new(&self.logging.level)
            .map_err(|_| invalid("logging.level contains an invalid filter directive"))?;
        if self.logging.file_enabled {
            validate_non_empty("logging.file_dir", &self.logging.file_dir)?;
            validate_non_empty("logging.file_prefix", &self.logging.file_prefix)?;
            validate_non_empty("logging.file_suffix", &self.logging.file_suffix)?;
        }

        validate_files(&self.files)?;
        validate_telegram(&self.telegram)?;
        validate_worker(&self.notification_worker, &self.telegram)?;

        if self.telegram.enabled || self.notification_worker.enabled {
            validate(
                self.telegram
                    .bot_token
                    .as_deref()
                    .is_some_and(|token| !token.trim().is_empty()),
                "telegram.bot_token is required when a Telegram component is enabled",
            )?;
        }

        Ok(ValidatedAppConfig(self))
    }
}

pub struct ValidatedAppConfig(AppConfig);

impl std::ops::Deref for ValidatedAppConfig {
    type Target = AppConfig;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Debug, thiserror::Error)]
#[error("invalid application configuration: {message}")]
pub struct ConfigValidationError {
    message: String,
}

fn validate_files(config: &FilesConfig) -> Result<(), ConfigValidationError> {
    validate(
        config.max_file_size_bytes > 0,
        "files.max_file_size_bytes must be positive",
    )?;
    validate(
        config.max_file_size_bytes <= i64::MAX as u64,
        "files.max_file_size_bytes is too large",
    )?;
    validate(
        config.max_files_per_assignment > 0,
        "files.max_files_per_assignment must be positive",
    )?;
    validate(
        config.max_files_per_assignment <= 10,
        "files.max_files_per_assignment must not exceed 10",
    )?;
    validate(
        config.max_files_per_submission > 0,
        "files.max_files_per_submission must be positive",
    )?;
    validate_positive("files.transfer_timeout_ms", config.transfer_timeout_ms)?;
    validate(
        config.max_concurrent_transfers > 0,
        "files.max_concurrent_transfers must be positive",
    )?;
    validate(
        config.max_concurrent_transfers <= tokio::sync::Semaphore::MAX_PERMITS,
        "files.max_concurrent_transfers is too large",
    )?;

    let size = usize::try_from(config.max_file_size_bytes)
        .map_err(|_| invalid("files.max_file_size_bytes does not fit this platform"))?;
    size.checked_mul(config.max_concurrent_transfers)
        .ok_or_else(|| invalid("total concurrent file transfer size is too large"))?;
    Ok(())
}

fn validate_telegram(config: &TelegramConfig) -> Result<(), ConfigValidationError> {
    let url = reqwest::Url::parse(&config.api_base_url)
        .map_err(|_| invalid("telegram.api_base_url must be a valid URL"))?;
    validate(
        matches!(url.scheme(), "http" | "https"),
        "telegram.api_base_url must use HTTP or HTTPS",
    )?;
    validate_positive("telegram.request_timeout_ms", config.request_timeout_ms)?;
    validate(
        config.polling_timeout_ms >= 1000 && config.polling_timeout_ms.is_multiple_of(1000),
        "telegram.polling_timeout_ms must use whole seconds",
    )?;
    validate(
        config.polling_timeout_ms / 1000 <= u64::from(u32::MAX),
        "telegram.polling_timeout_ms is too large",
    )?;
    validate(
        config.rate_limit_burst > 0,
        "telegram.rate_limit_burst must be positive",
    )?;
    validate_positive(
        "telegram.rate_limit_replenish_ms",
        config.rate_limit_replenish_ms,
    )?;
    validate(
        (1..=1024).contains(&config.max_concurrent_updates),
        "telegram.max_concurrent_updates must be between 1 and 1024",
    )?;
    validate_positive("telegram.shutdown_grace_ms", config.shutdown_grace_ms)
}

fn validate_worker(
    config: &NotificationWorkerConfig,
    telegram: &TelegramConfig,
) -> Result<(), ConfigValidationError> {
    validate_positive(
        "notification_worker.poll_interval_ms",
        config.poll_interval_ms,
    )?;
    validate_positive(
        "notification_worker.lease_duration_ms",
        config.lease_duration_ms,
    )?;
    validate(
        config.lease_duration_ms > telegram.request_timeout_ms,
        "notification_worker.lease_duration_ms must exceed telegram.request_timeout_ms",
    )?;
    validate(
        config.max_attempts > 0,
        "notification_worker.max_attempts must be positive",
    )?;
    validate_positive(
        "notification_worker.retry_initial_ms",
        config.retry_initial_ms,
    )?;
    validate(
        config.retry_max_ms >= config.retry_initial_ms,
        "notification_worker.retry_max_ms must not be less than retry_initial_ms",
    )?;
    validate_positive(
        "notification_worker.shutdown_grace_ms",
        config.shutdown_grace_ms,
    )
}

fn validate_positive(field: &'static str, value: u64) -> Result<(), ConfigValidationError> {
    validate(value > 0, format!("{field} must be positive"))
}

fn validate_non_empty(field: &'static str, value: &str) -> Result<(), ConfigValidationError> {
    validate(
        !value.trim().is_empty(),
        format!("{field} must not be empty"),
    )
}

fn validate(condition: bool, message: impl Into<String>) -> Result<(), ConfigValidationError> {
    condition.then_some(()).ok_or_else(|| invalid(message))
}

fn invalid(message: impl Into<String>) -> ConfigValidationError {
    ConfigValidationError {
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_app_config() {
        let config: AppConfig = ::config::Config::builder()
            .add_source(::config::File::from_str(
                r#"
                [app]
                name = "hw-bot"

                [server]
                host = "127.0.0.1"
                port = 3000
                request_timeout_ms = 10000

                [database]
                url = "postgres://postgres:postgres@localhost:5432/hw-bot"
                max_connections = 5
                acquire_timeout_ms = 3000
                statement_timeout_ms = 5000
                run_migrations = true
                run_bootstrap = false
                bootstrap_file = "tools/deploy/bootstrap.production.json"

                [logging]
                level = "hw_bot=info,tower_http=info,sqlx=warn"
                format = "compact"
                file_enabled = true
                file_dir = "logs/hw-bot"
                file_prefix = "hw-bot"
                file_suffix = "log"
                rotation = "daily"
                also_stdout = true

                [files]
                max_file_size_bytes = 5242880
                max_files_per_assignment = 5
                max_files_per_submission = 1
                transfer_timeout_ms = 60000
                max_concurrent_transfers = 4

                [telegram]
                enabled = false
                api_base_url = "https://api.telegram.org"
                request_timeout_ms = 5000
                polling_timeout_ms = 10000
                rate_limit_burst = 8
                rate_limit_replenish_ms = 1000
                max_concurrent_updates = 32
                shutdown_grace_ms = 10000

                [notification_worker]
                enabled = false
                poll_interval_ms = 500
                lease_duration_ms = 15000
                max_attempts = 10
                retry_initial_ms = 1000
                retry_max_ms = 300000
                shutdown_grace_ms = 10000
                "#,
                ::config::FileFormat::Toml,
            ))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();

        assert_eq!(config.app.name, "hw-bot");
        assert_eq!(config.server.host, "127.0.0.1".parse::<IpAddr>().unwrap());
        assert_eq!(config.server.port, 3000);
        assert_eq!(config.server.request_timeout_ms, 10000);
        assert_eq!(
            config.database.url,
            "postgres://postgres:postgres@localhost:5432/hw-bot"
        );
        assert_eq!(config.database.max_connections, 5);
        assert_eq!(config.database.acquire_timeout_ms, 3000);
        assert_eq!(config.database.statement_timeout_ms, 5000);
        assert!(config.database.run_migrations);
        assert!(!config.database.run_bootstrap);
        assert_eq!(
            config.database.bootstrap_file,
            PathBuf::from("tools/deploy/bootstrap.production.json")
        );
        assert_eq!(
            config.logging.level,
            "hw_bot=info,tower_http=info,sqlx=warn"
        );
        assert!(matches!(config.logging.format, LoggingFormat::Compact));
        assert!(config.logging.file_enabled);
        assert_eq!(config.logging.file_dir, "logs/hw-bot");
        assert_eq!(config.logging.file_prefix, "hw-bot");
        assert_eq!(config.logging.file_suffix, "log");
        assert!(matches!(config.logging.rotation, LogRotation::Daily));
        assert!(config.logging.also_stdout);
        assert_eq!(config.files.max_file_size_bytes, 5_242_880);
        assert_eq!(config.files.max_files_per_assignment, 5);
        assert_eq!(config.files.max_files_per_submission, 1);
        assert_eq!(config.files.transfer_timeout_ms, 60_000);
        assert_eq!(config.files.max_concurrent_transfers, 4);
        assert_eq!(config.telegram.api_base_url, "https://api.telegram.org");
        assert!(config.telegram.bot_token.is_none());
        assert!(!config.telegram.enabled);
        assert_eq!(config.telegram.polling_timeout_ms, 10000);
        assert_eq!(config.telegram.rate_limit_burst, 8);
        assert_eq!(config.telegram.rate_limit_replenish_ms, 1000);
        assert_eq!(config.telegram.max_concurrent_updates, 32);
        assert_eq!(config.telegram.shutdown_grace_ms, 10000);
        assert!(!config.notification_worker.enabled);
        assert_eq!(config.notification_worker.max_attempts, 10);
    }

    #[test]
    fn server_host_accepts_loopback_ip_addresses() {
        for host in ["127.0.0.1", "127.0.0.2", "::1"] {
            let config = server_config(host).unwrap();
            assert!(config.host.is_loopback(), "host: {host}");
        }
    }

    #[test]
    fn server_host_rejects_non_loopback_ip_addresses() {
        for host in [
            "0.0.0.0",
            "::",
            "10.0.0.1",
            "192.168.1.10",
            "8.8.8.8",
            "fe80::1",
            "::ffff:127.0.0.1",
        ] {
            let error = server_config(host).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("server.host must be a loopback IP address"),
                "host: {host}, error: {error}"
            );
        }
    }

    #[test]
    fn validation_rejects_enabled_telegram_without_token() {
        let mut config = app_config();
        config.telegram.enabled = true;

        let error = config.validate().err().expect("config must be rejected");

        assert!(error.to_string().contains("telegram.bot_token"));
    }

    #[test]
    fn validation_rejects_zero_database_connections() {
        let mut config = app_config();
        config.database.max_connections = 0;

        let error = config.validate().err().expect("config must be rejected");

        assert!(error.to_string().contains("database.max_connections"));
    }

    #[test]
    fn validation_rejects_unknown_fields() {
        let error = ::config::Config::builder()
            .add_source(::config::File::from_str(
                r#"
                [server]
                host = "127.0.0.1"
                port = 3000
                request_timeout_ms = 10000
                typo = true
                "#,
                ::config::FileFormat::Toml,
            ))
            .build()
            .unwrap()
            .try_deserialize::<ServerConfig>()
            .unwrap_err();

        assert!(error.to_string().contains("unknown field"));
    }

    fn app_config() -> AppConfig {
        ::config::Config::builder()
            .add_source(::config::File::with_name("config/default"))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap()
    }

    #[test]
    fn server_host_rejects_hostnames_and_socket_syntax() {
        for host in ["localhost", "[::1]", "http://127.0.0.1"] {
            assert!(server_config(host).is_err(), "host: {host}");
        }
    }

    fn server_config(host: &str) -> Result<ServerConfig, ::config::ConfigError> {
        ::config::Config::builder()
            .set_override("host", host.to_owned())?
            .set_override("port", 3000)?
            .set_override("request_timeout_ms", 10000)?
            .build()?
            .try_deserialize()
    }
}
