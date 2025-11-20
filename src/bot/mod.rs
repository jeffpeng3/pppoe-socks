use crate::pppoe::manager::PPPoEManager;
use anyhow::{Error, Result};
use poise::serenity_prelude as serenity;
use std::sync::Arc;

pub struct Data {
    pub manager: Arc<PPPoEManager>,
}

pub type Context<'a> = poise::Context<'a, Data, Error>;

/// Autocomplete function for interface names
async fn autocomplete_interface<'a>(ctx: Context<'a>, partial: &'a str) -> Vec<String> {
    let manager = &ctx.data().manager;
    let stats = manager.get_all_stats().await;

    stats
        .keys()
        .filter(|name| name.starts_with(partial))
        .map(|s| s.to_string())
        .collect()
}

/// Get the status of all PPPoE interfaces
#[poise::command(slash_command)]
pub async fn status(ctx: Context<'_>) -> Result<()> {
    let manager = &ctx.data().manager;
    let stats = manager.get_all_stats().await;

    let mut response = String::from("**PPPoE Connection Status**\n");
    for (interface, info) in stats {
        response.push_str(&format!("**{}**: ", interface));
        if let Some(ip) = info.local_ip {
            response.push_str(&format!("Connected (IP: {})", ip));
            if let Some(connected_at) = info.connected_at {
                let duration = chrono::Utc::now() - connected_at;
                response.push_str(&format!(", Uptime: {}s", duration.num_seconds()));
            }

            if info.is_healthy {
                response.push_str(" ✅");
            } else {
                response.push_str(&format!(" ⚠️ (Failures: {})", info.consecutive_failures));
            }

            if let Some(last_check) = info.last_health_check {
                let since_check = chrono::Utc::now() - last_check;
                response.push_str(&format!(
                    " [Last check: {}s ago]",
                    since_check.num_seconds()
                ));
            }
        } else {
            response.push_str("Disconnected");
        }
        response.push('\n');
    }

    ctx.say(response).await?;
    Ok(())
}

/// Reconnect a specific PPPoE interface
#[poise::command(slash_command)]
pub async fn reconnect(
    ctx: Context<'_>,
    #[description = "Interface name (e.g., ppp0)"]
    #[autocomplete = "autocomplete_interface"]
    interface: String,
) -> Result<()> {
    let manager = &ctx.data().manager;
    match manager.reconnect_client(&interface).await {
        Ok(_) => {
            ctx.say(format!("Reconnecting {}...", interface)).await?;
        }
        Err(e) => {
            ctx.say(format!("Failed to reconnect {}: {}", interface, e))
                .await?;
        }
    }
    Ok(())
}

/// Disconnect a specific PPPoE interface
#[poise::command(slash_command)]
pub async fn disconnect(
    ctx: Context<'_>,
    #[description = "Interface name (e.g., ppp0)"]
    #[autocomplete = "autocomplete_interface"]
    interface: String,
) -> Result<()> {
    let manager = &ctx.data().manager;
    match manager.disconnect_client(&interface).await {
        Ok(_) => {
            ctx.say(format!("Disconnecting {}...", interface)).await?;
        }
        Err(e) => {
            ctx.say(format!("Failed to disconnect {}: {}", interface, e))
                .await?;
        }
    }
    Ok(())
}

/// Connect a specific PPPoE interface
#[poise::command(slash_command)]
pub async fn connect(
    ctx: Context<'_>,
    #[description = "Interface name (e.g., ppp0)"]
    #[autocomplete = "autocomplete_interface"]
    interface: String,
) -> Result<()> {
    let manager = &ctx.data().manager;
    match manager.connect_client(&interface).await {
        Ok(_) => {
            ctx.say(format!("Connecting {}...", interface)).await?;
        }
        Err(e) => {
            ctx.say(format!("Failed to connect {}: {}", interface, e))
                .await?;
        }
    }
    Ok(())
}

/// Trigger a health check for a specific PPPoE interface
#[poise::command(slash_command)]
pub async fn healthcheck(
    ctx: Context<'_>,
    #[description = "Interface name (e.g., ppp0)"]
    #[autocomplete = "autocomplete_interface"]
    interface: String,
) -> Result<()> {
    let manager = &ctx.data().manager;
    ctx.say(format!("Running health check for {}...", interface))
        .await?;

    let is_healthy = manager.check_health(&interface).await;
    manager.update_health_status(&interface, is_healthy).await;

    if is_healthy {
        ctx.say(format!("✅ {} is healthy", interface)).await?;
    } else {
        ctx.say(format!("⚠️ {} is unhealthy", interface)).await?;
    }
    Ok(())
}

pub async fn start_bot(
    token: String,
    guild_id: Option<u64>,
    manager: Arc<PPPoEManager>,
) -> Result<()> {
    let intents = serenity::GatewayIntents::non_privileged();

    let framework = poise::Framework::builder()
        .options(poise::FrameworkOptions {
            commands: vec![
                status(),
                reconnect(),
                disconnect(),
                connect(),
                healthcheck(),
            ],
            ..Default::default()
        })
        .setup(move |ctx, _ready, framework| {
            Box::pin(async move {
                if let Some(guild_id) = guild_id {
                    poise::builtins::register_in_guild(
                        ctx,
                        &framework.options().commands,
                        serenity::GuildId::new(guild_id),
                    )
                    .await?;
                } else {
                    poise::builtins::register_globally(ctx, &framework.options().commands).await?;
                }
                Ok(Data { manager })
            })
        })
        .build();

    let client = serenity::ClientBuilder::new(token, intents)
        .framework(framework)
        .await;
    client?.start().await?;
    Ok(())
}
