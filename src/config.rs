use std::net::IpAddr;
use std::path::PathBuf;

use serde::{Deserialize, Deserializer};

#[derive(Clone, Deserialize)]
pub struct AppConfig {
    pub app: AppInfoConfig,
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub logging: LoggingConfig,
    pub telegram: TelegramConfig,
    pub notification_worker: NotificationWorkerConfig,
}

#[derive(Clone, Debug, Deserialize)]
pub struct AppInfoConfig {
    pub name: String,
}

#[derive(Clone, Debug, Deserialize)]
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

#[derive(Clone, Deserialize)]
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
                bootstrap_file = "bootstrap/data.json"

                [logging]
                level = "hw_bot=info,tower_http=info,sqlx=warn"
                format = "compact"
                file_enabled = true
                file_dir = "logs/hw-bot"
                file_prefix = "hw-bot"
                file_suffix = "log"
                rotation = "daily"
                also_stdout = true

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
            PathBuf::from("bootstrap/data.json")
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
