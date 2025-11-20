use anyhow::{Context, Result, anyhow};
use chrono::{Local, Timelike};
use log::debug;
use std::env;

#[derive(Debug, Clone)]
pub struct IpRotationConfig {
    pub rotation_time: String,
    pub wait_seconds: u32,
    pub health_check_enabled: bool,
    pub health_check_interval_secs: u64,
    pub health_check_failure_threshold: u32,
    pub health_check_target: String,
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub username: String,
    pub password: String,
    pub session_count: u16,
    pub ip_rotation: IpRotationConfig,
    pub logger_level: String,
    pub discord_token: String,
    pub discord_guild_id: Option<u64>,
    pub gateway: String,
}

impl AppConfig {
    pub fn load() -> Result<Self> {
        dotenvy::dotenv().unwrap_or_default();

        let username = env::var("PPPOE_USERNAME").context("PPPOE_USERNAME not set")?;
        let password = env::var("PPPOE_PASSWORD").context("PPPOE_PASSWORD not set")?;

        let session_count: u16 = env::var("PPPOE_SESSION_COUNT")
            .unwrap_or_else(|_| "1".to_string())
            .parse()
            .context("Invalid PPPOE_SESSION_COUNT")?;

        if session_count > 7 {
            return Err(anyhow!("PPPOE_SESSION_COUNT cannot exceed 7"));
        }

        let discord_token = env::var("DISCORD_TOKEN").context("DISCORD_TOKEN not set")?;
        let discord_guild_id = env::var("DISCORD_GUILD_ID")
            .ok()
            .and_then(|id| id.parse().ok());

        let logger_level = env::var("GOST_LOG_LEVEL").unwrap_or_else(|_| "warn".to_string());

        let rotation_time = env::var("IP_ROTATION_TIME").context("IP_ROTATION_TIME not set")?;
        let wait_seconds_str =
            env::var("IP_ROTATION_WAIT_SECONDS").context("IP_ROTATION_WAIT_SECONDS not set")?;

        if !is_valid_time_format(&rotation_time) && rotation_time.parse::<u32>().is_err() {
            return Err(anyhow!(
                "Invalid IP_ROTATION_TIME: {}. Must be in HH:MM format or a positive integer representing minutes",
                rotation_time
            ));
        }

        let wait_seconds = wait_seconds_str
            .parse::<u32>()
            .context("Invalid IP_ROTATION_WAIT_SECONDS: Must be a non-negative integer")?;

        let health_check_enabled = env::var("HEALTH_CHECK_ENABLED")
            .unwrap_or_else(|_| "true".to_string())
            .parse()
            .unwrap_or(true);

        let health_check_interval_secs = env::var("HEALTH_CHECK_INTERVAL")
            .unwrap_or_else(|_| "30".to_string())
            .parse()
            .unwrap_or(30);

        let health_check_failure_threshold = env::var("HEALTH_CHECK_THRESHOLD")
            .unwrap_or_else(|_| "3".to_string())
            .parse()
            .unwrap_or(3);

        let health_check_target =
            env::var("HEALTH_CHECK_TARGET").unwrap_or_else(|_| "8.8.8.8".to_string());

        let gateway = env::var("GATEWAY").context("GATEWAY not set")?;

        let ip_rotation = IpRotationConfig {
            rotation_time,
            wait_seconds,
            health_check_enabled,
            health_check_interval_secs,
            health_check_failure_threshold,
            health_check_target,
        };

        Ok(Self {
            username,
            password,
            session_count,
            ip_rotation,
            logger_level,
            discord_token,
            discord_guild_id,
            gateway,
        })
    }
}

fn is_valid_time_format(time: &str) -> bool {
    let parts: Vec<&str> = time.split(':').collect();
    if parts.len() != 2 {
        return false;
    }
    let hour = parts[0].parse::<u32>();
    let minute = parts[1].parse::<u32>();
    matches!((hour, minute), (Ok(h), Ok(m)) if h < 24 && m < 60)
}

pub fn time_string_to_sec(time_str: &str) -> Result<i64> {
    let parts: Vec<&str> = time_str.split(':').collect();
    if parts.len() != 2 {
        return Err(anyhow!("Invalid time format: {}", time_str));
    }
    let hour: u32 = parts[0]
        .parse()
        .map_err(|_| anyhow!("Invalid hour: {}", parts[0]))?;
    let minute: u32 = parts[1]
        .parse()
        .map_err(|_| anyhow!("Invalid minute: {}", parts[1]))?;
    let local_now = Local::now();

    let next_time = local_now
        .with_hour(hour)
        .unwrap()
        .with_minute(minute)
        .unwrap()
        .with_second(0)
        .unwrap();
    let next_time = if next_time < local_now {
        next_time + chrono::Duration::days(1)
    } else {
        next_time
    };
    debug!(
        "Current local time: {}",
        local_now.format("%Y-%m-%d %H:%M:%S")
    );
    debug!(
        "Next rotation time: {}",
        next_time.format("%Y-%m-%d %H:%M:%S")
    );
    Ok(next_time.timestamp() - local_now.timestamp())
}
