use anyhow::Result;
use log::error;
use log::info;
use std::env::var;
use tokio::process::Command;

pub async fn init_route(dry_run: bool) -> Result<()> {
    if dry_run {
        info!("[DRY-RUN] Skipping route initialization");
        return Ok(());
    }
    for i in 0..8 {
        let table_id = 100 + i;
        Command::new("ip")
            .args([
                "rule",
                "add",
                "iif",
                format!("tun{i}").as_str(),
                "table",
                &table_id.to_string(),
                "prio",
                &table_id.to_string(),
            ])
            .output()
            .await
            .map_err(|e| {
                error!("Failed to add rule: {}", e);
                e
            })?;
    }

    Command::new("ip")
        .args([
            "route",
            "add",
            "default",
            "via",
            var("GATEWAY")?.as_str(),
            "dev",
            "eth0",
            "table",
            "100",
        ])
        .output()
        .await
        .map_err(|e| {
            error!("Failed to add default route: {}", e);
            e
        })?;

    Ok(())
}
