use anyhow::Result;
use chrono::{DateTime, Utc};

use log::{debug, error, info, trace};
use std::collections::HashMap;
use std::sync::Arc;
use sysinfo::Networks;
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::{self, Duration};

use crate::core::config::{IpRotationConfig, time_string_to_sec};
use crate::pppoe::client::PPPoEClient;

#[derive(Debug, Clone, Default)]
pub struct ConnectionInfo {
    pub connected_at: Option<DateTime<Utc>>,
    pub local_ip: Option<String>,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub packets_sent: u64,
    pub packets_received: u64,
    pub uptime_seconds: u64,
    pub send_rate_bps: u64,
    pub receive_rate_bps: u64,
}

pub struct PPPoEManager {
    data: Arc<Mutex<HashMap<String, ConnectionInfo>>>,
    clients: Arc<Mutex<Vec<Arc<Mutex<PPPoEClient>>>>>,
    config: IpRotationConfig,
    stats_task: Mutex<Option<JoinHandle<()>>>,
}

impl PPPoEManager {
    pub fn new(config: IpRotationConfig) -> Arc<Self> {
        info!("IP Rotation Config: {:?}", config);

        Arc::new(Self {
            data: Arc::new(Mutex::new(HashMap::new())),
            clients: Arc::new(Mutex::new(Vec::new())),
            config,
            stats_task: Mutex::new(None),
        })
    }

    pub fn create_clients(
        manager: Arc<Self>,
        username: String,
        password: String,
        count: u16,
    ) -> Vec<Arc<Mutex<PPPoEClient>>> {
        let mut clients = Vec::new();
        for i in 0..count {
            let client = PPPoEClient::new(
                username.clone(),
                password.clone(),
                format!("ppp{}", i),
                Arc::clone(&manager),
            );
            clients.push(client);
        }
        clients
    }

    pub async fn set_clients(&self, clients: Vec<Arc<Mutex<PPPoEClient>>>) {
        *self.clients.lock().await = clients;
    }

    pub async fn start_stats_task(manager: Arc<Self>) {
        let data = Arc::clone(&manager.data);
        let task = tokio::spawn(async move {
            let mut networks = Networks::new();
            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                networks.refresh(true);
                let mut data_lock = data.lock().await;
                for (interface, info) in data_lock.iter_mut() {
                    if let Some(net) = networks.get(interface) {
                        info.send_rate_bps = net.transmitted() * 8;
                        info.receive_rate_bps = net.received() * 8;
                        info.bytes_received = net.total_received();
                        info.bytes_sent = net.total_transmitted();
                        info.packets_received = net.total_packets_received();
                        info.packets_sent = net.total_packets_transmitted();
                        if let Some(connected_at) = info.connected_at {
                            info.uptime_seconds = (Utc::now() - connected_at).num_seconds() as u64;
                        }
                        trace!("Traffic stats updated for interface {}", interface);
                    }
                }
                drop(data_lock);
            }
        });
        *manager.stats_task.lock().await = Some(task);
    }

    pub async fn update_connection_info(
        &self,
        interface: &str,
        local_ip: Option<String>,
        connected_at: Option<DateTime<Utc>>,
    ) {
        let mut data = self.data.lock().await;
        let info = data
            .entry(interface.to_string())
            .or_insert(ConnectionInfo::default());

        if let Some(ip) = local_ip.clone() {
            info!("{}: {}", interface, ip);
        }
        let idx = interface.chars().last().unwrap().to_digit(10).unwrap();
        self.add_default_route(interface, 101 + idx).await.unwrap();
        info.local_ip = local_ip;
        info.connected_at = connected_at;
    }

    pub async fn add_default_route(&self, interface: &str, table_id: u32) -> Result<()> {
        Command::new("ip")
            .args([
                "route",
                "add",
                "default",
                "dev",
                interface,
                "table",
                &table_id.to_string(),
            ])
            .output()
            .await
            .map_err(|e| {
                error!("Failed to add default route: {}", e);
                e
            })?;
        Ok(())
    }

    pub async fn get_stats(&self, interface: &str) -> Option<ConnectionInfo> {
        let data = self.data.lock().await;
        data.get(interface).cloned()
    }

    pub async fn stop_all(&self) {
        let clients = self.clients.lock().await.clone();
        for client in clients.iter() {
            let client = Arc::clone(client);
            PPPoEClient::disconnect(client).await;
        }
        debug!("All PPPoE clients have been disconnected");
    }

    pub async fn start_all(&self) {
        let clients = self.clients.lock().await.clone();
        for client in clients.iter() {
            let client = Arc::clone(client);
            PPPoEClient::connect(Arc::clone(&client)).await;
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        }
        debug!("All PPPoE clients have been connected");
    }

    pub async fn rotate_ips(&self) {
        debug!("Starting IP rotation for all clients");

        self.stop_all().await;

        debug!(
            "Waiting {} seconds before reconnecting",
            self.config.wait_seconds
        );
        time::sleep(Duration::from_secs(self.config.wait_seconds as u64)).await;

        self.start_all().await;

        debug!("Reconnection phase completed for all clients");
        debug!("IP rotation completed for all clients");
    }

    fn calculate_next_rotation_seconds(&self) -> i64 {
        if let Ok(interval) = self.config.rotation_time.parse::<i64>() {
            return interval * 60;
        }

        time_string_to_sec(&self.config.rotation_time)
    }

    pub async fn serve(&self) {
        debug!("Starting PPPoE Manager");
        self.start_all().await;
        if self.config.rotation_time == "0" {
            info!("IP rotation disabled");
            return;
        }
        loop {
            let secs = self.calculate_next_rotation_seconds();
            info!("Next IP rotation in {} seconds", secs);
            time::sleep(Duration::from_secs(secs as u64)).await;
            self.rotate_ips().await;
        }
    }
}
