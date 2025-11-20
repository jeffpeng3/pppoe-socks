use chrono::Utc;
use log::{debug, info, trace, warn};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::pppoe::manager::{ConnectionInfo, PPPoEManager};

pub struct PPPoEClient {
    username: String,
    password: String,
    pub interface: String,
    enable: Arc<Mutex<bool>>,
    pub connected: Arc<Mutex<bool>>,
    pub stats_manager: Arc<PPPoEManager>,
    pppd: Option<Child>,
    monitor_task: Option<JoinHandle<()>>,
}

impl PPPoEClient {
    pub fn new(
        username: String,
        password: String,
        interface: String,
        stats_manager: Arc<PPPoEManager>,
    ) -> Arc<Mutex<Self>> {
        Arc::new(Mutex::new(Self {
            username,
            password,
            interface,
            enable: Arc::new(Mutex::new(false)),
            connected: Arc::new(Mutex::new(false)),
            stats_manager,
            pppd: None,
            monitor_task: None,
        }))
    }

    pub async fn connect(client: Arc<Mutex<Self>>) {
        let mut c = client.lock().await;
        *c.enable.lock().await = true;
        if *c.connected.lock().await {
            warn!("Already connected, no need to reconnect");
            return;
        }
        let client_clone = Arc::clone(&client);
        let monitor = tokio::spawn(async move {
            PPPoEClient::pppd_monitor_static(client_clone).await;
        });
        c.monitor_task = Some(monitor);
    }

    #[allow(dead_code)]
    pub async fn disconnect(client: Arc<Mutex<Self>>) {
        let mut c = client.lock().await;
        if !*c.connected.lock().await {
            warn!("Not connected, no need to disconnect");
        }
        *c.enable.lock().await = false;
        if let Some(pppd) = &mut c.pppd {
            pppd.kill().await.expect("Failed to kill pppd");
        }
        info!("PPPoE connection disconnected");
    }

    pub async fn get_traffic_stats(&self) -> Option<ConnectionInfo> {
        self.stats_manager.get_stats(&self.interface).await
    }

    async fn pppd_monitor_static(client: Arc<Mutex<PPPoEClient>>) {
        loop {
            let c = client.lock().await;
            let cmd = vec![
                "pppd".to_string(),
                "pty".to_string(),
                "pppoe".to_string(),
                "noauth".to_string(),
                "nodetach".to_string(),
                "usepeerdns".to_string(),
                "ifname".to_string(),
                c.interface.clone(),
                "user".to_string(),
                c.username.clone(),
                "password".to_string(),
                c.password.clone(),
            ];
            trace!("Starting PPPoE connection, command: {}", cmd.join(" "));
            let mut child = Command::new("pppd")
                .args(&cmd[1..])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .expect("Failed to start pppd");
            let stdout = child.stdout.take().unwrap();
            drop(c); // release lock
            {
                let mut c = client.lock().await;
                c.pppd = Some(child);
            }

            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            while let Ok(n) = reader.read_line(&mut line).await {
                if n == 0 {
                    break;
                }
                let trimmed = line.trim();
                trace!("PPPoE output line: {}", trimmed);
                if trimmed.contains("local  IP address") {
                    let parts: Vec<&str> = trimmed.split_whitespace().collect();
                    if parts.len() >= 4 {
                        let local_ip = parts[3].to_string();
                        let c = client.lock().await;
                        c.stats_manager
                            .update_connection_info(
                                &c.interface,
                                Some(local_ip.clone()),
                                Some(Utc::now()),
                            )
                            .await;
                        debug!("Parsed local IP address: {}", local_ip);
                        *c.connected.lock().await = true;
                        debug!("PPPoE connection successful");
                    }
                }
                line.clear();
            }
            {
                info!("PPPoE disconnected");
                let c = client.lock().await;
                *c.connected.lock().await = false;
            }
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            let enable = *client.lock().await.enable.lock().await;
            if !enable {
                break;
            } else {
                info!("Reconnecting PPPoE...");
            }
        }
    }
}
