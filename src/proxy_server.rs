use log::{debug, error};
use serde_json::{Value, json};
use std::process::Stdio;
use std::sync::Arc;
use std::env;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

pub struct ProxyServer {
    process: Option<Child>,
    config_json: String,
    guard_task: Option<JoinHandle<()>>,
}

fn respawn_proxy_server(proxy: Arc<Mutex<ProxyServer>>) {
    tokio::spawn(Box::pin(ProxyServer::start(proxy)));
}

fn proxy_service(index: u16, interface: &str) -> Value {
    json!({
      "name": format!("if{}-proxy", index),
      "addr": format!(":{}", 8080 + index),
      "bypass": "local-bypass",
      "handler": {
        "type": "auto",
        "metadata": {
          "udp": "true",
          "udpBufferSize": "65565"
        }
      },
      "listener": {
        "type": "tcp",
      },
      "metadata": {
        "interface": interface,
      }
    })
}

fn tun_service(index: u16, interface: &str) -> Value {
    json!({
      "name": format!("if{}-tun", index),
      "addr": format!(":{}", 8880 + index),
      "handler": {
        "type": "tun",
        "metadata": {
          "bufferSize": 65535,
        }
      },
      "bypass": "local-bypass",
      "listener": {
        "type": "tun",
        "metadata": {
          "name": interface,
          "net": format!("192.168.{}.1/24", 100 + index),
        }
      }
    })
}

impl ProxyServer {
    pub fn new(session_count: u16, logger_level: String) -> Arc<Mutex<Self>> {
        let bypass = json!([
          {
            "name": "local-bypass",
            "matchers": [
              "127.0.0.1/8",
              "10.0.0.0/8",
              "172.16.0.0/12",
              "192.168.0.0/16",
              "::1/128",
              "fc00::/7"
            ]
          }
        ]);

        let mut services = Vec::new();

        services.push(proxy_service(0, "eth0"));
        services.push(tun_service(0, "tun0"));

        for i in 0..session_count {
            services.push(proxy_service(i + 1, &format!("ppp{}", i)));
            services.push(tun_service(i + 1, &format!("tun{}", i + 1)));
        }

        let config = json!({
            "services": services,
            "bypasses": bypass,
            "api": {
                "addr": ":18080"
            },
            "metrics": {
                "addr": ":9000"
            },
            "log": {
                "format": "text",
                "level": logger_level
            },
        });
        let config_json = config.to_string();

        Arc::new(Mutex::new(Self {
            process: None,
            config_json,
            guard_task: None,
        }))
    }

    pub async fn start(proxy: Arc<Mutex<Self>>) {
        let guard_proxy = Arc::clone(&proxy);
        let mut p = proxy.lock().await;
        debug!("Starting proxy service with JSON config: {}", p.config_json);
        let verbose = env::var("PROXY_VERBOSE").unwrap_or_else(|_| "false".to_string()) == "true"
            || env::var("PROXY_VERBOSE").unwrap_or_else(|_| "0".to_string()) == "1";

        let stdio = if verbose { Stdio::inherit } else { Stdio::null };

        let child = Command::new("./gost")
            .arg("-C")
            .arg(&p.config_json)
            .stdout(stdio())
            .stderr(stdio())
            .spawn()
            .expect("Failed to start proxy");

        p.process = Some(child);

        let guard = tokio::spawn(async move {
            ProxyServer::guard(guard_proxy).await;
        });
        p.guard_task = Some(guard);
    }

    async fn guard(mutex_proxy: Arc<Mutex<Self>>) {
        let child_to_wait = {
            let mut proxy = mutex_proxy.lock().await;
            proxy.process.take()
        };
        error!("Proxy service guard started");
        if let Some(mut child) = child_to_wait {
            let _exit_status = child.wait().await.expect("Failed to wait for child");
            debug!("Proxy service exited abnormally, restarting...");
            respawn_proxy_server(mutex_proxy);
        }
    }
}
