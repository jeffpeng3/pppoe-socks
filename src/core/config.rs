use anyhow::{Context, Result};
use chrono::{Local, Timelike};
use log::{debug, error};
use std::env;

#[derive(Debug, Clone)]
pub struct IpRotationConfig {
    pub rotation_time: String,
    pub wait_seconds: u32,
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
    pub dry_run: bool,
}

impl AppConfig {
    pub fn load() -> Result<Self> {
        dotenvy::dotenv().unwrap_or_default();

        let username = env::var("PPPOE_USERNAME").context("PPPOE_USERNAME not set")?;
        let password = env::var("PPPOE_PASSWORD").context("PPPOE_PASSWORD not set")?;

        let session_count: u16 = env::var("PPPOE_SESSION_COUNT")
            .unwrap_or_else(|_| "1".to_string())
            .parse()
            .unwrap_or(1);

        let discord_token = env::var("DISCORD_TOKEN").context("DISCORD_TOKEN not set")?;
        let discord_guild_id = env::var("DISCORD_GUILD_ID")
            .ok()
            .and_then(|id| id.parse().ok());

        let dry_run = env::var("DRY_RUN")
            .unwrap_or_else(|_| "false".to_string())
            .parse()
            .unwrap_or(false);

        let logger_level = env::var("RUST_LOG").unwrap_or_else(|_| "warn".to_string());

        let rotation_time = env::var("IP_ROTATION_TIME").expect("IP_ROTATION_TIME not set");
        let wait_seconds_str =
            env::var("IP_ROTATION_WAIT_SECONDS").expect("IP_ROTATION_WAIT_SECONDS not set");

        match &rotation_time {
            t if is_valid_time_format(t) => {}
            t if t.parse::<u32>().is_ok() => {}
            _ => {
                error!(
                    "Invalid IP_ROTATION_TIME: {}. Must be in HH:MM format or a positive integer representing minutes",
                    rotation_time
                );
                panic!("Invalid IP_ROTATION_TIME format");
            }
        }

        let wait_seconds = wait_seconds_str
            .parse::<u32>()
            .expect("Invalid IP_ROTATION_WAIT_SECONDS: Must be a non-negative integer");

        let ip_rotation = IpRotationConfig {
            rotation_time,
            wait_seconds,
        };

        Ok(Self {
            username,
            password,
            session_count,
            ip_rotation,
            logger_level,
            discord_token,
            discord_guild_id,
            dry_run,
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

pub fn time_string_to_sec(time_str: &str) -> i64 {
    let parts: Vec<&str> = time_str.split(':').collect();
    if parts.len() != 2 {
        panic!("Invalid time format: {}", time_str);
    }
    let hour: u32 = parts[0]
        .parse()
        .unwrap_or_else(|_| panic!("Invalid hour: {}", parts[0]));
    let minute: u32 = parts[1]
        .parse()
        .unwrap_or_else(|_| panic!("Invalid minute: {}", parts[1]));
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
    next_time.timestamp() - local_now.timestamp()
}
