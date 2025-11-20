use anyhow::Result;
use chrono::{DateTime, Utc};

use log::{debug, error, info, trace};
use std::collections::HashMap;
use std::sync::Arc;
use sysinfo::Networks;
use tokio::process::Command;
use tokio::sync::{Mutex, mpsc};
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

#[derive(Debug)]
pub enum ClientCommand {
    Connect,
    Disconnect,
    Reconnect,
}

#[derive(Debug)]
pub enum PpmsEvent {
    IpUpdated {
        interface: String,
        local_ip: Option<String>,
        connected_at: Option<DateTime<Utc>>,
    },
    Disconnected {
        interface: String,
    },
}

pub struct PPPoEManager {
    data: Arc<Mutex<HashMap<String, ConnectionInfo>>>,
    client_controls: Arc<Mutex<HashMap<String, mpsc::Sender<ClientCommand>>>>,
    config: IpRotationConfig,
    stats_task: Mutex<Option<JoinHandle<()>>>,
    event_receiver: Mutex<Option<mpsc::Receiver<PpmsEvent>>>,
}

impl PPPoEManager {
    pub fn new(config: IpRotationConfig) -> Arc<Self> {
        info!("IP Rotation Config: {:?}", config);

        Arc::new(Self {
            data: Arc::new(Mutex::new(HashMap::new())),
            client_controls: Arc::new(Mutex::new(HashMap::new())),
            config,
            stats_task: Mutex::new(None),
            event_receiver: Mutex::new(None),
        })
    }

    pub async fn set_event_receiver(&self, receiver: mpsc::Receiver<PpmsEvent>) {
        *self.event_receiver.lock().await = Some(receiver);
    }

    pub async fn start_clients(
        &self,
        username: String,
        password: String,
        count: u16,
        event_sender: mpsc::Sender<PpmsEvent>,
    ) {
        let mut controls = self.client_controls.lock().await;
        for i in 0..count {
            let interface = format!("ppp{}", i);
            let (cmd_tx, cmd_rx) = mpsc::channel(32);

            let client = PPPoEClient::new(
                username.clone(),
                password.clone(),
                interface.clone(),
                event_sender.clone(),
                cmd_rx,
            );

            tokio::spawn(client.run());
            controls.insert(interface, cmd_tx);
        }
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

    pub async fn stop_all(&self) {
        let controls = self.client_controls.lock().await;
        for (interface, tx) in controls.iter() {
            if let Err(e) = tx.send(ClientCommand::Disconnect).await {
                error!("Failed to send Disconnect to {}: {}", interface, e);
            }
        }
        debug!("Sent Disconnect command to all clients");
    }

    pub async fn start_all(&self) {
        let controls = self.client_controls.lock().await;
        for (interface, tx) in controls.iter() {
            if let Err(e) = tx.send(ClientCommand::Connect).await {
                error!("Failed to send Connect to {}: {}", interface, e);
            }
            // Stagger connections slightly?
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }
        debug!("Sent Connect command to all clients");
    }

    pub async fn reconnect_client(&self, interface: &str) -> Result<()> {
        let controls = self.client_controls.lock().await;
        if let Some(tx) = controls.get(interface) {
            tx.send(ClientCommand::Reconnect)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to send Reconnect: {}", e))?;
            Ok(())
        } else {
            Err(anyhow::anyhow!("Interface {} not found", interface))
        }
    }

    pub async fn disconnect_client(&self, interface: &str) -> Result<()> {
        let controls = self.client_controls.lock().await;
        if let Some(tx) = controls.get(interface) {
            tx.send(ClientCommand::Disconnect)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to send Disconnect: {}", e))?;
            Ok(())
        } else {
            Err(anyhow::anyhow!("Interface {} not found", interface))
        }
    }

    pub async fn connect_client(&self, interface: &str) -> Result<()> {
        let controls = self.client_controls.lock().await;
        if let Some(tx) = controls.get(interface) {
            tx.send(ClientCommand::Connect)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to send Connect: {}", e))?;
            Ok(())
        } else {
            Err(anyhow::anyhow!("Interface {} not found", interface))
        }
    }

    pub async fn get_all_stats(&self) -> HashMap<String, ConnectionInfo> {
        let data = self.data.lock().await;
        data.clone()
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

        // Start event loop in a separate task?
        // Or run it here concurrently with rotation loop?
        // Let's spawn the event loop separately or use select!
        // But serve() is expected to block (it has a loop).

        // We need to run the event loop.
        // We need to run the event loop.
        // We can't easily clone &Self to Arc<Self> unless we are inside an Arc.
        // But serve takes &self.

        // Ideally, main.rs should spawn the event loop.
        // Let's add a run_event_loop method that takes Arc<Self>.

        self.start_all().await;
        if self.config.rotation_time == "0" {
            info!("IP rotation disabled");
            // Just wait forever
            loop {
                tokio::time::sleep(Duration::from_secs(3600)).await;
            }
        }
        loop {
            let secs = self.calculate_next_rotation_seconds();
            info!("Next IP rotation in {} seconds", secs);
            time::sleep(Duration::from_secs(secs as u64)).await;
            self.rotate_ips().await;
        }
    }

    pub async fn run_event_loop(self: Arc<Self>) {
        let mut receiver = self
            .event_receiver
            .lock()
            .await
            .take()
            .expect("Event receiver not set");
        info!("Event loop started");
        while let Some(event) = receiver.recv().await {
            match event {
                PpmsEvent::IpUpdated {
                    interface,
                    local_ip,
                    connected_at,
                } => {
                    self.update_connection_info(&interface, local_ip, connected_at)
                        .await;
                }
                PpmsEvent::Disconnected { interface } => {
                    self.update_connection_info(&interface, None, None).await;
                }
            }
        }
        info!("Event loop stopped");
    }
}
