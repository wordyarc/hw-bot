use std::net::IpAddr;
use std::num::{NonZeroU16, NonZeroU32, NonZeroU64, NonZeroUsize};
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;

use chrono_tz::Tz;
use serde::{Deserialize, Deserializer};

use crate::bot::MAX_CONCURRENT_UPDATES;
use crate::invites::InviteSigningSecret;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    pub app: AppInfoConfig,
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub logging: LoggingConfig,
    pub files: FilesConfig,
    pub ui: UiConfig,
    pub telegram: TelegramConfig,
    pub notification_worker: NotificationWorkerConfig,
    pub group_invites: GroupInvitesConfig,
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
    pub port: NonZeroU16,
    pub request_timeout_ms: NonZeroU64,
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
    pub max_connections: NonZeroU32,
    pub acquire_timeout_ms: NonZeroU64,
    pub statement_timeout_ms: NonZeroU64,
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
    pub max_file_size_bytes: NonZeroU64,
    pub max_files_per_assignment: NonZeroUsize,
    pub max_files_per_submission: NonZeroUsize,
    pub transfer_timeout_ms: NonZeroU64,
    pub max_concurrent_transfers: NonZeroUsize,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiConfig {
    #[serde(deserialize_with = "deserialize_timezone")]
    pub timezone: Tz,
}

fn deserialize_timezone<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Tz, D::Error> {
    String::deserialize(deserializer)?
        .trim()
        .parse()
        .map_err(|_| {
            serde::de::Error::custom("ui.timezone must be an IANA name like Asia/Vladivostok")
        })
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TelegramConfig {
    pub enabled: bool,
    pub api_base_url: String,
    pub bot_token: Option<String>,
    pub request_timeout_ms: NonZeroU64,
    pub polling_timeout_ms: NonZeroU64,
    pub rate_limit_burst: NonZeroU32,
    pub rate_limit_replenish_ms: NonZeroU64,
    pub max_concurrent_updates: NonZeroUsize,
    pub shutdown_grace_ms: NonZeroU64,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroupInvitesConfig {
    #[serde(deserialize_with = "deserialize_signing_secret")]
    pub invite_signing_secret: InviteSigningSecret,
}

fn deserialize_signing_secret<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<InviteSigningSecret, D::Error> {
    InviteSigningSecret::from_base64url(&String::deserialize(deserializer)?)
        .map_err(|_| serde::de::Error::custom("group_invites.invite_signing_secret must be 32 random bytes in base64url without padding"))
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NotificationWorkerConfig {
    pub enabled: bool,
    pub poll_interval_ms: NonZeroU64,
    pub lease_duration_ms: NonZeroU64,
    pub max_attempts: NonZeroU32,
    pub retry_initial_ms: NonZeroU64,
    pub retry_max_ms: NonZeroU64,
    pub shutdown_grace_ms: NonZeroU64,
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
    pub fn shutdown_grace(&self) -> Duration {
        Duration::from_millis(
            self.server
                .request_timeout_ms
                .max(self.telegram.shutdown_grace_ms)
                .max(self.notification_worker.shutdown_grace_ms)
                .get(),
        )
    }

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

        sqlx::postgres::PgConnectOptions::from_str(&self.database.url)
            .map_err(|_| invalid("database.url must be a valid PostgreSQL URL"))?;

        tracing_subscriber::EnvFilter::try_new(&self.logging.level)
            .map_err(|_| invalid("logging.level contains an invalid filter directive"))?;
        if self.logging.file_enabled {
            validate_non_empty("logging.file_dir", &self.logging.file_dir)?;
            validate_non_empty("logging.file_prefix", &self.logging.file_prefix)?;
            validate_non_empty("logging.file_suffix", &self.logging.file_suffix)?;
        }

        validate_files(&self.files)?;
        validate_telegram(&self.telegram)?;
        validate_worker(&self.notification_worker)?;
        validate_limit_coherence(&self)?;

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

    fn deref(&self) -> &AppConfig {
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
        config.max_file_size_bytes.get() <= i64::MAX as u64,
        "files.max_file_size_bytes is too large",
    )?;
    validate(
        config.max_files_per_assignment.get() <= 10,
        "files.max_files_per_assignment must not exceed 10",
    )?;
    validate(
        config.max_concurrent_transfers.get() <= tokio::sync::Semaphore::MAX_PERMITS,
        "files.max_concurrent_transfers is too large",
    )?;

    let size = usize::try_from(config.max_file_size_bytes.get())
        .map_err(|_| invalid("files.max_file_size_bytes does not fit this platform"))?;
    size.checked_mul(config.max_concurrent_transfers.get())
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
    validate(
        config.polling_timeout_ms.get().is_multiple_of(1000),
        "telegram.polling_timeout_ms must use whole seconds",
    )?;
    validate(
        config.polling_timeout_ms.get() / 1000 <= u64::from(u32::MAX),
        "telegram.polling_timeout_ms is too large",
    )?;
    validate(
        config.max_concurrent_updates.get() <= MAX_CONCURRENT_UPDATES,
        format!("telegram.max_concurrent_updates must not exceed {MAX_CONCURRENT_UPDATES}"),
    )
}

fn validate_worker(config: &NotificationWorkerConfig) -> Result<(), ConfigValidationError> {
    validate(
        config.retry_max_ms >= config.retry_initial_ms,
        "notification_worker.retry_max_ms must not be less than retry_initial_ms",
    )
}

fn validate_limit_coherence(config: &AppConfig) -> Result<(), ConfigValidationError> {
    let database_round_trip = config
        .database
        .acquire_timeout_ms
        .get()
        .saturating_add(config.database.statement_timeout_ms.get());

    validate(
        config.server.request_timeout_ms.get() >= database_round_trip,
        "server.request_timeout_ms must cover database.acquire_timeout_ms + \
         database.statement_timeout_ms",
    )?;
    validate(
        config.notification_worker.lease_duration_ms.get()
            >= database_round_trip
                .saturating_mul(2)
                .saturating_add(config.telegram.request_timeout_ms.get()),
        "notification_worker.lease_duration_ms must cover 2 * (database.acquire_timeout_ms + \
         database.statement_timeout_ms) + telegram.request_timeout_ms",
    )?;
    validate(
        Duration::from_millis(config.files.transfer_timeout_ms.get()) <= config.shutdown_grace(),
        "files.transfer_timeout_ms must not exceed the effective shutdown grace",
    )
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
                transfer_timeout_ms = 20000
                max_concurrent_transfers = 4

                [ui]
                timezone = "Asia/Vladivostok"

                [telegram]
                enabled = false
                api_base_url = "https://api.telegram.org"
                request_timeout_ms = 5000
                polling_timeout_ms = 10000
                rate_limit_burst = 8
                rate_limit_replenish_ms = 1000
                max_concurrent_updates = 32
                shutdown_grace_ms = 20000

                [group_invites]
                invite_signing_secret = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8"

                [notification_worker]
                enabled = false
                poll_interval_ms = 500
                lease_duration_ms = 25000
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
        assert_eq!(config.server.port.get(), 3000);
        assert_eq!(config.server.request_timeout_ms.get(), 10000);
        assert_eq!(
            config.database.url,
            "postgres://postgres:postgres@localhost:5432/hw-bot"
        );
        assert_eq!(config.database.max_connections.get(), 5);
        assert_eq!(config.database.acquire_timeout_ms.get(), 3000);
        assert_eq!(config.database.statement_timeout_ms.get(), 5000);
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
        assert_eq!(config.files.max_file_size_bytes.get(), 5_242_880);
        assert_eq!(config.files.max_files_per_assignment.get(), 5);
        assert_eq!(config.files.max_files_per_submission.get(), 1);
        assert_eq!(config.files.transfer_timeout_ms.get(), 20_000);
        assert_eq!(config.files.max_concurrent_transfers.get(), 4);
        assert_eq!(config.ui.timezone, Tz::Asia__Vladivostok);
        assert_eq!(config.telegram.api_base_url, "https://api.telegram.org");
        assert!(config.telegram.bot_token.is_none());
        assert!(!config.telegram.enabled);
        assert_eq!(config.telegram.polling_timeout_ms.get(), 10000);
        assert_eq!(config.telegram.rate_limit_burst.get(), 8);
        assert_eq!(config.telegram.rate_limit_replenish_ms.get(), 1000);
        assert_eq!(config.telegram.max_concurrent_updates.get(), 32);
        assert_eq!(config.telegram.shutdown_grace_ms.get(), 20000);
        assert!(!config.notification_worker.enabled);
        assert_eq!(config.notification_worker.max_attempts.get(), 10);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn deserialization_rejects_a_missing_invite_signing_secret() {
        let error = config_builder()
            .build()
            .unwrap()
            .try_deserialize::<AppConfig>()
            .err()
            .expect("config must be rejected");
        assert!(error.to_string().contains("invite_signing_secret"));
    }

    #[test]
    fn deserialization_rejects_an_invite_signing_secret_of_the_wrong_size() {
        for secret in ["", "AAEC", "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8g"] {
            let error = config_builder()
                .set_override("group_invites.invite_signing_secret", secret)
                .unwrap()
                .build()
                .unwrap()
                .try_deserialize::<AppConfig>()
                .err()
                .unwrap_or_else(|| panic!("secret {secret:?} must be rejected"));

            assert!(error.to_string().contains("group_invites"), "{secret:?}");
        }
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
    fn deserialization_rejects_an_unknown_timezone() {
        for timezone in ["UTC+10", "+10:00", "Vladivostok", ""] {
            let error = config_builder()
                .set_override("ui.timezone", timezone)
                .unwrap()
                .set_override("group_invites.invite_signing_secret", TEST_SECRET)
                .unwrap()
                .build()
                .unwrap()
                .try_deserialize::<AppConfig>()
                .err()
                .unwrap_or_else(|| panic!("timezone {timezone:?} must be rejected"));

            assert!(
                error.to_string().contains("ui.timezone"),
                "timezone {timezone:?}"
            );
        }
    }

    #[test]
    fn the_installation_timezone_defaults_to_vladivostok() {
        let config = app_config();

        assert_eq!(config.ui.timezone, Tz::Asia__Vladivostok);
    }

    #[test]
    fn deserialization_rejects_zero_limits_with_the_field_name() {
        for field in [
            "server.port",
            "server.request_timeout_ms",
            "database.max_connections",
            "database.acquire_timeout_ms",
            "database.statement_timeout_ms",
            "files.max_file_size_bytes",
            "files.max_files_per_assignment",
            "files.max_files_per_submission",
            "files.transfer_timeout_ms",
            "files.max_concurrent_transfers",
            "telegram.request_timeout_ms",
            "telegram.polling_timeout_ms",
            "telegram.rate_limit_burst",
            "telegram.rate_limit_replenish_ms",
            "telegram.max_concurrent_updates",
            "telegram.shutdown_grace_ms",
            "notification_worker.poll_interval_ms",
            "notification_worker.lease_duration_ms",
            "notification_worker.max_attempts",
            "notification_worker.retry_initial_ms",
            "notification_worker.retry_max_ms",
            "notification_worker.shutdown_grace_ms",
        ] {
            let error = config_builder()
                .set_override("group_invites.invite_signing_secret", TEST_SECRET)
                .unwrap()
                .set_override(field, 0)
                .unwrap()
                .build()
                .unwrap()
                .try_deserialize::<AppConfig>()
                .err()
                .expect("zero must be rejected")
                .to_string();
            assert!(error.contains(field), "{field}: {error}");
            assert!(error.contains("nonzero"), "{field}: {error}");
        }
    }

    #[test]
    fn the_effective_shutdown_grace_is_the_largest_contribution() {
        let mut config = app_config();
        config.server.request_timeout_ms = NonZeroU64::new(1000).unwrap();
        config.telegram.shutdown_grace_ms = NonZeroU64::new(7000).unwrap();
        config.notification_worker.shutdown_grace_ms = NonZeroU64::new(3000).unwrap();

        assert_eq!(config.shutdown_grace(), Duration::from_millis(7000));
    }

    #[test]
    fn default_configuration_satisfies_every_limit_relation() {
        app_config()
            .validate()
            .unwrap_or_else(|error| panic!("default configuration must be coherent: {error}"));
    }

    #[test]
    fn validation_rejects_a_request_timeout_shorter_than_one_database_round_trip() {
        let mut config = app_config();
        config.database.statement_timeout_ms = NonZeroU64::new(10_000).unwrap();

        let error = config.validate().err().expect("config must be rejected");

        assert!(
            error.to_string().contains("server.request_timeout_ms"),
            "{error}"
        );
    }

    #[test]
    fn validation_rejects_a_lease_shorter_than_one_delivery() {
        let mut config = app_config();
        config.database.statement_timeout_ms = NonZeroU64::new(10_000).unwrap();
        // Проверка HTTP-timeout не должна перехватить ошибку, ради которой написан этот тест.
        config.server.request_timeout_ms = NonZeroU64::new(30_000).unwrap();

        let error = config.validate().err().expect("config must be rejected");

        assert!(
            error
                .to_string()
                .contains("notification_worker.lease_duration_ms"),
            "{error}"
        );
    }

    #[test]
    fn lease_covers_both_database_trips_even_when_each_trip_fits() {
        let mut config = app_config();
        config.notification_worker.lease_duration_ms = NonZeroU64::new(13_000).unwrap();
        assert!(config.clone().validate().is_err());
        config.notification_worker.lease_duration_ms = NonZeroU64::new(21_000).unwrap();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validation_rejects_a_file_transfer_timeout_beyond_the_shutdown_grace() {
        let mut config = app_config();
        config.files.transfer_timeout_ms =
            NonZeroU64::new(config.shutdown_grace().as_millis() as u64 + 1).unwrap();

        let error = config.validate().err().expect("config must be rejected");

        assert!(
            error.to_string().contains("files.transfer_timeout_ms"),
            "{error}"
        );
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

    const TEST_SECRET: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8";

    fn config_builder() -> ::config::ConfigBuilder<::config::builder::DefaultState> {
        ::config::Config::builder().add_source(::config::File::with_name("config/default"))
    }

    fn app_config() -> AppConfig {
        config_builder()
            .set_override("group_invites.invite_signing_secret", TEST_SECRET)
            .unwrap()
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
