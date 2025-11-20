use env_logger::Builder;
use log::{error, info, trace};
use std::env;
use std::sync::Arc;
use tokio::process::Command;
use tokio::sync::Mutex;

mod pppoe_client;
mod pppoe_manager;
mod proxy_server;
mod route_manager;

use pppoe_client::PPPoEClient;
use pppoe_manager::PPPoEManager;
use proxy_server::ProxyServer;
use route_manager::init_route;

#[tokio::main]
async fn main() {
    Command::new("nft")
        .arg("-f")
        .arg("/etc/nftables.conf")
        .status()
        .await
        .expect("Failed to execute nft command");

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

    let pppoe_manager = PPPoEManager::new();
    PPPoEManager::start_stats_task(Arc::clone(&pppoe_manager)).await;

    let username = env::var("PPPOE_USERNAME").expect("PPPOE_USERNAME not set");
    let password = env::var("PPPOE_PASSWORD").expect("PPPOE_PASSWORD not set");
    let session_count: u16 = env::var("PPPOE_SESSION_COUNT")
        .unwrap_or_else(|_| "1".to_string())
        .parse()
        .unwrap_or(1);
    let logger_level = env::var("RUST_LOG").unwrap_or_else(|_| "warn".to_string());

    let mut clients: Vec<Arc<Mutex<PPPoEClient>>> = Vec::new();
    for i in 0..session_count {
        let client = PPPoEClient::new(
            username.clone(),
            password.clone(),
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

    let proxy = ProxyServer::new(session_count, logger_level);
    ProxyServer::start(proxy).await;
    loop {
        for client in &clients {
            let c = client.lock().await;
            if *c.connected.lock().await
                && let Some(stats) = c.get_traffic_stats().await
            {
                trace!("{} {:?}", c.interface, stats);
            }
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    }
}
