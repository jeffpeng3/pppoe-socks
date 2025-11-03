use anyhow::Result;
use chrono::{DateTime, Local, Timelike, Utc};
use core::panic;
use log::{debug, error, info, trace};
use std::collections::HashMap;
use std::env;
use std::sync::Arc;
use sysinfo::Networks;
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::{self, Duration};

use crate::pppoe_client::PPPoEClient;

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

#[derive(Debug, Clone)]
pub struct IpRotationConfig {
    pub rotation_time: String,
    pub wait_seconds: u32,
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

fn time_string_to_sec(time_str: &str) -> i64 {
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

pub struct PPPoEManager {
    data: Arc<Mutex<HashMap<String, ConnectionInfo>>>,
    clients: Arc<Mutex<Vec<Arc<Mutex<PPPoEClient>>>>>,
    config: IpRotationConfig,
    stats_task: Mutex<Option<JoinHandle<()>>>,
}

impl PPPoEManager {
    pub fn new() -> Arc<Self> {
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

        let config = IpRotationConfig {
            rotation_time,
            wait_seconds,
        };

        info!("IP Rotation Config: {:?}", config);

        Arc::new(Self {
            data: Arc::new(Mutex::new(HashMap::new())),
            clients: Arc::new(Mutex::new(Vec::new())),
            config,
            stats_task: Mutex::new(None),
        })
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
