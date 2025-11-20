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
    should_be_connected: bool,
    reconnect_attempts: u32,
    max_reconnect_attempts: u32,
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
            should_be_connected: false,
            reconnect_attempts: 0,
            max_reconnect_attempts: 10, // 0 表示無限重試
        }
    }

    pub async fn run(mut self) {
        info!("PPPoE Client {} started", self.interface);

        // Initial connect
        self.should_be_connected = true;
        self.connect().await;

        loop {
            tokio::select! {
                Some(cmd) = self.command_receiver.recv() => {
                    match cmd {
                        ClientCommand::Connect => {
                            self.should_be_connected = true;
                            if self.pppd.is_none() {
                                self.reconnect_attempts = 0;
                                self.connect().await;
                            }
                        }
                        ClientCommand::Disconnect => {
                            self.should_be_connected = false;
                            self.reconnect_attempts = 0;
                            self.disconnect().await;
                        }
                        ClientCommand::Reconnect => {
                            self.should_be_connected = true;
                            self.reconnect_attempts = 0;
                            self.disconnect().await;
                            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                            self.connect().await;
                        }
                    }
                }
                // 監聽 pppd 進程退出
                Some(result) = async {
                    if let Some(ref mut child) = self.pppd {
                        child.wait().await.ok()
                    } else {
                        None
                    }
                } => {
                    info!("{}: pppd process exited with {:?}", self.interface, result);
                    self.pppd = None;

                    // 發送斷線事件
                    let _ = self.event_sender.send(PpmsEvent::Disconnected {
                        interface: self.interface.clone(),
                    }).await;

                    // 如果應該保持連線，則自動重連
                    if self.should_be_connected {
                        // 檢查是否超過最大重連次數（0 表示無限重試）
                        if self.max_reconnect_attempts == 0 || self.reconnect_attempts < self.max_reconnect_attempts {
                            self.reconnect_attempts += 1;

                            // Linear backoff: min(5 * N, 30) seconds
                            let delay = std::cmp::min(
                                5 * self.reconnect_attempts as u64,
                                30
                            );

                            info!(
                                "{}: Auto-reconnecting in {} seconds (attempt {}/{})",
                                self.interface,
                                delay,
                                self.reconnect_attempts,
                                if self.max_reconnect_attempts == 0 { "∞".to_string() } else { self.max_reconnect_attempts.to_string() }
                            );

                            tokio::time::sleep(tokio::time::Duration::from_secs(delay)).await;
                            self.connect().await;
                        } else {
                            error!(
                                "{}: Max reconnection attempts ({}) reached, giving up",
                                self.interface,
                                self.max_reconnect_attempts
                            );
                            self.should_be_connected = false;
                        }
                    } else {
                        info!("{}: Manual disconnect, not auto-reconnecting", self.interface);
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

            // 連線成功，重置重連計數器
            self.reconnect_attempts = 0;

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
                    let mut ip_obtained = false;
                    while let Ok(n) = reader.read_line(&mut line).await {
                        if n == 0 {
                            break;
                        }
                        let trimmed = line.trim();
                        if trimmed.contains("local  IP address") {
                            let parts: Vec<&str> = trimmed.split_whitespace().collect();
                            if parts.len() >= 4 {
                                let local_ip = parts[3].to_string();
                                ip_obtained = true;
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
                    // stdout 關閉表示 pppd 進程即將結束
                    // 斷線事件由 run() 中的進程監聽統一處理
                    if ip_obtained {
                        info!("{}: pppd stdout closed, connection likely lost", interface);
                    }
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
