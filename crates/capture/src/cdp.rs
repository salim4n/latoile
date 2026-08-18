//! Minimal Chrome DevTools Protocol transport and isolated Chromium process.

use crate::CaptureError;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tempfile::TempDir;
use tokio::net::TcpStream;
use tokio::process::{Child, Command};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message};

pub(super) fn find_browser(override_path: Option<&Path>) -> Result<PathBuf, CaptureError> {
    let candidates = override_path.into_iter().map(Path::to_path_buf).chain([
        PathBuf::from("/usr/bin/google-chrome-stable"),
        PathBuf::from("/usr/bin/google-chrome"),
        PathBuf::from("/usr/bin/chromium"),
        PathBuf::from("/usr/bin/chromium-browser"),
        PathBuf::from("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"),
    ]);
    candidates
        .into_iter()
        .find(|path| path.is_file())
        .ok_or(CaptureError::BrowserUnavailable)
}

pub(super) struct ChromeProcess {
    child: Child,
    profile: TempDir,
    port: u16,
}

impl ChromeProcess {
    pub(super) async fn launch(executable: &Path) -> Result<Self, CaptureError> {
        let profile = tempfile::Builder::new()
            .prefix("latoile-chrome-")
            .tempdir()
            .map_err(|error| CaptureError::Storage(error.to_string()))?;
        let mut child = Command::new(executable)
            .arg("--headless=new")
            .arg("--remote-debugging-port=0")
            .arg(format!("--user-data-dir={}", profile.path().display()))
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .arg("--disable-background-networking")
            .arg("--disable-component-update")
            .arg("--disable-domain-reliability")
            .arg("--disable-extensions")
            .arg("--disable-sync")
            .arg("--disable-gpu")
            .arg("--force-color-profile=srgb")
            .arg("--hide-scrollbars")
            .arg("--host-resolver-rules=MAP * 0.0.0.0")
            .arg("about:blank")
            .env_clear()
            .env("LANG", "C.UTF-8")
            .env("LC_ALL", "C.UTF-8")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|_| CaptureError::BrowserStartup)?;
        let active = profile.path().join("DevToolsActivePort");
        let mut port = None;
        for _ in 0..100 {
            if let Ok(text) = tokio::fs::read_to_string(&active).await {
                port = text
                    .lines()
                    .next()
                    .and_then(|value| value.parse::<u16>().ok());
                if port.is_some() {
                    break;
                }
            }
            if child.try_wait().ok().flatten().is_some() {
                return Err(CaptureError::BrowserStartup);
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        Ok(Self {
            child,
            profile,
            port: port.ok_or(CaptureError::BrowserStartup)?,
        })
    }

    pub(super) async fn page_websocket(&self) -> Result<String, CaptureError> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Target {
            #[serde(rename = "type")]
            kind: String,
            web_socket_debugger_url: Option<String>,
        }
        let url = format!("http://127.0.0.1:{}/json/list", self.port);
        for _ in 0..40 {
            if let Ok(response) = reqwest::get(&url).await {
                if let Ok(targets) = response.json::<Vec<Target>>().await {
                    if let Some(url) = targets
                        .into_iter()
                        .find(|target| target.kind == "page")
                        .and_then(|target| target.web_socket_debugger_url)
                    {
                        return Ok(url);
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        Err(CaptureError::BrowserStartup)
    }

    pub(super) async fn shutdown(&mut self) {
        let _ = self.child.kill().await;
        let _ = self.child.wait().await;
        let _ = self.profile.path();
    }
}

impl Drop for ChromeProcess {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

pub(super) struct CdpClient {
    socket: WebSocketStream<MaybeTlsStream<TcpStream>>,
    next_id: u64,
    allowed_origin: Option<String>,
}

impl CdpClient {
    pub(super) async fn connect(url: &str) -> Result<Self, CaptureError> {
        let (socket, _) = connect_async(url)
            .await
            .map_err(|error| CaptureError::Protocol(error.to_string()))?;
        Ok(Self {
            socket,
            next_id: 1,
            allowed_origin: None,
        })
    }

    pub(super) fn restrict_network_to(&mut self, origin: String) {
        self.allowed_origin = Some(origin);
    }

    pub(super) async fn call(
        &mut self,
        method: &str,
        params: Value,
    ) -> Result<Value, CaptureError> {
        let id = self.next_id;
        self.next_id += 1;
        self.socket
            .send(Message::Text(
                json!({"id": id, "method": method, "params": params})
                    .to_string()
                    .into(),
            ))
            .await
            .map_err(|error| CaptureError::Protocol(error.to_string()))?;
        loop {
            let message = tokio::time::timeout(Duration::from_secs(10), self.socket.next())
                .await
                .map_err(|_| CaptureError::Protocol(format!("{method} timed out")))?
                .ok_or_else(|| CaptureError::Protocol("browser connection closed".into()))?
                .map_err(|error| CaptureError::Protocol(error.to_string()))?;
            let text = match message {
                Message::Text(text) => text.to_string(),
                Message::Binary(bytes) => String::from_utf8(bytes.to_vec())
                    .map_err(|error| CaptureError::Protocol(error.to_string()))?,
                Message::Ping(bytes) => {
                    self.socket
                        .send(Message::Pong(bytes))
                        .await
                        .map_err(|error| CaptureError::Protocol(error.to_string()))?;
                    continue;
                }
                Message::Close(_) => {
                    return Err(CaptureError::Protocol("browser connection closed".into()));
                }
                _ => continue,
            };
            let value: Value = serde_json::from_str(&text)
                .map_err(|error| CaptureError::Protocol(error.to_string()))?;
            if self.handle_intercept(&value).await? {
                continue;
            }
            if value.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = value.get("error") {
                return Err(CaptureError::Protocol(format!("{method}: {error}")));
            }
            return Ok(value.get("result").cloned().unwrap_or(Value::Null));
        }
    }

    async fn handle_intercept(&mut self, event: &Value) -> Result<bool, CaptureError> {
        if event.get("method").and_then(Value::as_str) != Some("Fetch.requestPaused") {
            return Ok(false);
        }
        let request_id = event
            .pointer("/params/requestId")
            .and_then(Value::as_str)
            .ok_or_else(|| CaptureError::Protocol("intercepted request id is missing".into()))?;
        let url = event
            .pointer("/params/request/url")
            .and_then(Value::as_str)
            .ok_or_else(|| CaptureError::Protocol("intercepted request URL is missing".into()))?;
        let origin = self
            .allowed_origin
            .as_deref()
            .ok_or_else(|| CaptureError::Protocol("network interception has no policy".into()))?;
        let websocket_origin = origin.replacen("http://", "ws://", 1);
        let allowed = url == origin
            || url.starts_with(&format!("{origin}/"))
            || url == websocket_origin
            || url.starts_with(&format!("{websocket_origin}/"))
            || url.starts_with("data:")
            || url.starts_with("blob:")
            || url == "about:blank";
        let command_id = self.next_id;
        self.next_id += 1;
        let command = if allowed {
            json!({
                "id": command_id,
                "method": "Fetch.continueRequest",
                "params": {"requestId": request_id},
            })
        } else {
            json!({
                "id": command_id,
                "method": "Fetch.failRequest",
                "params": {"requestId": request_id, "errorReason": "BlockedByClient"},
            })
        };
        self.socket
            .send(Message::Text(command.to_string().into()))
            .await
            .map_err(|error| CaptureError::Protocol(error.to_string()))?;
        Ok(true)
    }
}
