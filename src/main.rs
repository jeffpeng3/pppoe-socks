use log::{error, info, trace};
use std::sync::Arc;
use tokio::process::Command;

mod core;
mod network;
mod pppoe;
mod proxy;

use crate::core::config::AppConfig;
use crate::core::logger;
use crate::network::route::init_route;
use crate::pppoe::manager::PPPoEManager;
use crate::proxy::server::ProxyServer;
use anyhow::{Context, Result};

#[tokio::main]
async fn main() -> Result<()> {
    setup_nft().await?;

    logger::init();

    let _ = init_route().await.map_err(|x| error!("{x:?}"));

    info!("Starting ppproxy Service");

    let config = AppConfig::load()?;

    let pppoe_manager = PPPoEManager::new(config.ip_rotation.clone());
    PPPoEManager::start_stats_task(Arc::clone(&pppoe_manager)).await;

    let clients = PPPoEManager::create_clients(
        Arc::clone(&pppoe_manager),
        config.username.clone(),
        config.password.clone(),
        config.session_count,
    );

    pppoe_manager.set_clients(clients.clone()).await;
    let pppoe_manager_clone = Arc::clone(&pppoe_manager);
    tokio::spawn(async move {
        pppoe_manager_clone.serve().await;
    });

    let proxy = ProxyServer::new(config.session_count, config.logger_level.clone());
    ProxyServer::start(Arc::clone(&proxy)).await;

    info!("Service started. Press Ctrl+C to stop.");

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("Shutting down...");
                break;
            }
            _ = tokio::time::sleep(tokio::time::Duration::from_secs(1)) => {
                for client in &clients {
                    let c = client.lock().await;
                    if *c.connected.lock().await
                        && let Some(stats) = c.get_traffic_stats().await
                    {
                        trace!("{} {:?}", c.interface, stats);
                    }
                }
            }
        }
    }

    info!("Stopping services...");
    pppoe_manager.stop_all().await;
    ProxyServer::stop(proxy).await;
    info!("Goodbye!");

    Ok(())
}

async fn setup_nft() -> Result<()> {
    Command::new("nft")
        .arg("-f")
        .arg("/etc/nftables.conf")
        .status()
        .await
        .context("Failed to execute nft command")?;
    Ok(())
}
