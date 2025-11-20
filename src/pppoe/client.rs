use chrono::Utc;
use log::{error, info};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

use crate::pppoe::manager::{ClientCommand, PpmsEvent};

pub struct PPPoEClient {
    username: String,
    password: String,
    pub interface: String,
    pppd: Option<Child>,
    event_sender: mpsc::Sender<PpmsEvent>,
    command_receiver: mpsc::Receiver<ClientCommand>,
    dry_run: bool,
}

impl PPPoEClient {
    pub fn new(
        username: String,
        password: String,
        interface: String,
        event_sender: mpsc::Sender<PpmsEvent>,
        command_receiver: mpsc::Receiver<ClientCommand>,
        dry_run: bool,
    ) -> Self {
        Self {
            username,
            password,
            interface,
            pppd: None,
            event_sender,
            command_receiver,
            dry_run,
        }
    }

    pub async fn run(mut self) {
        info!("PPPoE Client {} started", self.interface);

        // Initial connect
        self.connect().await;

        loop {
            tokio::select! {
                Some(cmd) = self.command_receiver.recv() => {
                    match cmd {
                        ClientCommand::Connect => {
                            if self.pppd.is_none() {
                                self.connect().await;
                            }
                        }
                        ClientCommand::Disconnect => {
                            self.disconnect().await;
                        }
                        ClientCommand::Reconnect => {
                            self.disconnect().await;
                            // Wait a bit?
                            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                            self.connect().await;
                        }
                    }
                }
            }
        }
    }

    async fn connect(&mut self) {
        info!("Connecting {} (dry_run: {})", self.interface, self.dry_run);

        if self.dry_run {
            // Dry-run 模式：模擬連線成功
            let interface = self.interface.clone();
            let event_sender = self.event_sender.clone();

            // 從介面名稱生成假 IP (例如 ppp0 -> 10.0.0.1)
            let num: u8 = self
                .interface
                .trim_start_matches("ppp")
                .parse()
                .unwrap_or(0);
            let fake_ip = format!("10.0.{}.1", num);

            info!(
                "[DRY-RUN] Simulating connection for {} with IP {}",
                interface, fake_ip
            );

            // 延遲模擬連線建立時間
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

            let _ = event_sender
                .send(PpmsEvent::IpUpdated {
                    interface: interface.clone(),
                    local_ip: Some(fake_ip),
                    connected_at: Some(Utc::now()),
                })
                .await;

            return;
        }

        let cmd = vec![
            "pppd".to_string(),
            "pty".to_string(),
            "pppoe".to_string(),
            "noauth".to_string(),
            "nodetach".to_string(),
            "usepeerdns".to_string(),
            "ifname".to_string(),
            self.interface.clone(),
            "user".to_string(),
            self.username.clone(),
            "password".to_string(),
            self.password.clone(),
        ];

        match Command::new("pppd")
            .args(&cmd[1..])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
        {
            Ok(mut child) => {
                let stdout = child.stdout.take().unwrap();
                self.pppd = Some(child);

                let interface = self.interface.clone();
                let event_sender = self.event_sender.clone();

                tokio::spawn(async move {
                    let mut reader = BufReader::new(stdout);
                    let mut line = String::new();
                    while let Ok(n) = reader.read_line(&mut line).await {
                        if n == 0 {
                            break;
                        }
                        let trimmed = line.trim();
                        if trimmed.contains("local  IP address") {
                            let parts: Vec<&str> = trimmed.split_whitespace().collect();
                            if parts.len() >= 4 {
                                let local_ip = parts[3].to_string();
                                let _ = event_sender
                                    .send(PpmsEvent::IpUpdated {
                                        interface: interface.clone(),
                                        local_ip: Some(local_ip),
                                        connected_at: Some(Utc::now()),
                                    })
                                    .await;
                            }
                        }
                        line.clear();
                    }
                    let _ = event_sender
                        .send(PpmsEvent::Disconnected {
                            interface: interface.clone(),
                        })
                        .await;
                });
            }
            Err(e) => {
                error!("Failed to start pppd for {}: {}", self.interface, e);
            }
        }
    }

    async fn disconnect(&mut self) {
        if let Some(mut child) = self.pppd.take() {
            let _ = child.kill().await;
        }
    }
}
