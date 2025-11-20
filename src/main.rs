use env_logger::Builder;
use log::{error, info, trace};
use std::sync::Arc;
use tokio::process::Command;
use tokio::sync::Mutex;

mod config;
mod pppoe_client;
mod pppoe_manager;
mod proxy_server;
mod route_manager;

use config::AppConfig;
use pppoe_client::PPPoEClient;
use pppoe_manager::PPPoEManager;
use proxy_server::ProxyServer;
use route_manager::init_route;

use anyhow::{Context, Result};

#[tokio::main]
async fn main() -> Result<()> {
    Command::new("nft")
        .arg("-f")
        .arg("/etc/nftables.conf")
        .status()
        .await
        .context("Failed to execute nft command")?;

    dotenvy::dotenv().unwrap_or_default();

    Builder::from_env(env_logger::Env::default())
        .format(|buf, record| {
            use chrono::Local;
            use env_logger::fmt::style::{AnsiColor, Style};
            use std::io::Write;

            let subtle = Style::new().fg_color(Some(AnsiColor::BrightBlack.into()));
            let level_style = buf.default_level_style(record.level());

            writeln!(
                buf,
                "{subtle}[{subtle:#}{} {level_style}{:<5}{level_style:#} {}{subtle}]{subtle:#} {}",
                Local::now().format("%m-%d %H:%M:%S%:::z"),
                record.level(),
                record.module_path().unwrap_or("<unknown>"),
                record.args(),
            )
        })
        .init();

    let _ = init_route().await.map_err(|x| error!("{x:?}"));

    info!("Starting ppproxy Service");

    let config = AppConfig::load()?;

    let pppoe_manager = PPPoEManager::new(config.ip_rotation.clone());
    PPPoEManager::start_stats_task(Arc::clone(&pppoe_manager)).await;

    let mut clients: Vec<Arc<Mutex<PPPoEClient>>> = Vec::new();
    for i in 0..config.session_count {
        let client = PPPoEClient::new(
            config.username.clone(),
            config.password.clone(),
            format!("ppp{}", i),
            Arc::clone(&pppoe_manager),
        );
        clients.push(client);
    }

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
