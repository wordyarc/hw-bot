use serde::Deserialize;

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
    pub host: String,
    pub port: u16,
}

#[derive(Clone, Debug, Deserialize)]
pub struct DatabaseConfig {
    pub url: String,
    pub max_connections: u32,
    pub run_migrations: bool,
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

                [database]
                url = "postgres://postgres:postgres@localhost:5432/hw-bot"
                max_connections = 5
                run_migrations = true

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
        assert_eq!(config.server.host, "127.0.0.1");
        assert_eq!(config.server.port, 3000);
        assert_eq!(
            config.database.url,
            "postgres://postgres:postgres@localhost:5432/hw-bot"
        );
        assert_eq!(config.database.max_connections, 5);
        assert!(config.database.run_migrations);
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
        assert_eq!(config.telegram.shutdown_grace_ms, 10000);
        assert!(!config.notification_worker.enabled);
        assert_eq!(config.notification_worker.max_attempts, 10);
    }
}
