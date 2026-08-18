//! Hermetic GitHub tests: a hand-rolled TCP listener plays scripted HTTP
//! responses on 127.0.0.1 and records what was asked. No framework, no
//! network beyond loopback, no real token.

use super::*;
use latoile_core::ports::PortError;
use std::collections::{HashMap, VecDeque};
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

// --- The mock ---------------------------------------------------------------

struct Recorded {
    request_line: String,
    headers: HashMap<String, String>,
    body: String,
}

struct Mock {
    addr: SocketAddr,
    requests: Arc<Mutex<Vec<Recorded>>>,
}

impl Mock {
    /// One scripted `(status, body)` per expected request, answered in order.
    async fn start(scripts: Vec<(u16, String)>) -> Self {
        let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = requests.clone();
        let mut scripts: VecDeque<(u16, String)> = scripts.into();

        tokio::spawn(async move {
            while let Some((status, body)) = scripts.pop_front() {
                let accept = tokio::time::timeout(Duration::from_secs(10), listener.accept()).await;
                let Ok(Ok((mut socket, _))) = accept else {
                    break;
                };
                let request = read_request(&mut socket).await;
                recorded.lock().unwrap().push(request);
                let reason = match status {
                    200 | 201 => "OK",
                    401 => "Unauthorized",
                    404 => "Not Found",
                    422 => "Unprocessable Entity",
                    _ => "Status",
                };
                let response = format!(
                    "HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\n\
                     content-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
            }
        });

        Self { addr, requests }
    }

    fn take_requests(&self) -> Vec<Recorded> {
        std::mem::take(&mut *self.requests.lock().unwrap())
    }
}

/// Headers up to the blank line, then exactly Content-Length bytes of body.
async fn read_request(socket: &mut tokio::net::TcpStream) -> Recorded {
    let mut raw = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        let read = socket.read(&mut chunk).await.unwrap_or(0);
        assert!(read > 0, "connection closed mid-request");
        raw.extend_from_slice(&chunk[..read]);
        if let Some(pos) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos + 4;
        }
    };
    let head = String::from_utf8_lossy(&raw[..header_end]).to_string();
    let mut lines = head.lines();
    let request_line = lines.next().unwrap_or_default().to_string();
    let headers: HashMap<String, String> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_lowercase(), v.trim().to_string()))
        .collect();
    let wanted: usize = headers
        .get("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut body = raw[header_end..].to_vec();
    while body.len() < wanted {
        let read = socket.read(&mut chunk).await.unwrap_or(0);
        assert!(read > 0, "connection closed mid-body");
        body.extend_from_slice(&chunk[..read]);
    }
    Recorded {
        request_line,
        headers,
        body: String::from_utf8_lossy(&body[..wanted]).to_string(),
    }
}

// --- Fixtures ---------------------------------------------------------------

/// A trivial in-memory SecretStore — simpler than a vault for these tests.
struct MemSecrets(HashMap<String, String>);

impl SecretStore for MemSecrets {
    async fn get(&self, name: &str) -> Result<Option<String>, PortError> {
        Ok(self.0.get(name).cloned())
    }
    async fn put(&self, _name: &str, _value: &str) -> Result<(), PortError> {
        unimplemented!()
    }
}

fn github(mock: &Mock, token: Option<&str>) -> GitHub<MemSecrets> {
    let mut secrets = HashMap::new();
    if let Some(token) = token {
        secrets.insert(DEFAULT_TOKEN_NAME.to_string(), token.to_string());
    }
    GitHub::new(
        GitHubConfig {
            api_base: format!("http://{}", mock.addr),
            ..GitHubConfig::default()
        },
        MemSecrets(secrets),
        GitHub::<MemSecrets>::default_http(),
    )
}

// --- Tests ------------------------------------------------------------------

#[tokio::test]
async fn list_repos_maps_the_picker_fields() {
    let mock = Mock::start(vec![(
        200,
        r#"[{"full_name":"salim4n/mon-app","description":"Mon app","private":true},{"full_name":"salim4n/empty","description":null,"private":false}]"#.into(),
    )])
    .await;
    let gh = github(&mock, Some("tok123"));

    let repos = GitHubClient::list_repos(&gh).await.unwrap();
    assert_eq!(
        repos,
        vec![
            RepoInfo {
                full_name: "salim4n/mon-app".into(),
                description: Some("Mon app".into()),
                private: true,
            },
            RepoInfo {
                full_name: "salim4n/empty".into(),
                description: None,
                private: false,
            },
        ]
    );

    let sent = mock.take_requests();
    assert_eq!(sent.len(), 1);
    assert!(
        sent[0].request_line.starts_with("GET /user/repos"),
        "{}",
        sent[0].request_line
    );
    assert_eq!(
        sent[0].headers.get("authorization").unwrap(),
        "Bearer tok123"
    );
}

#[tokio::test]
async fn open_pull_request_returns_its_url() {
    let mock = Mock::start(vec![(
        201,
        r#"{"html_url":"https://github.com/salim4n/mon-app/pull/3"}"#.into(),
    )])
    .await;
    let gh = github(&mock, Some("tok123"));

    let url = GitHubClient::open_pull_request(&gh, "salim4n/mon-app", "work", "main")
        .await
        .unwrap();
    assert_eq!(url, "https://github.com/salim4n/mon-app/pull/3");

    let sent = mock.take_requests();
    assert!(sent[0]
        .request_line
        .starts_with("POST /repos/salim4n/mon-app/pulls"));
    let body: serde_json::Value = serde_json::from_str(&sent[0].body).unwrap();
    assert_eq!(body["head"], "work");
    assert_eq!(body["base"], "main");
}

#[tokio::test]
async fn a_401_is_an_auth_error() {
    let mock = Mock::start(vec![(401, r#"{"message":"Bad credentials"}"#.into())]).await;
    let gh = github(&mock, Some("expired"));
    let err = GitHubClient::list_repos(&gh).await.unwrap_err();
    assert!(err.to_string().contains("refused the token"), "{err}");
}

#[tokio::test]
async fn a_404_is_a_not_found() {
    let mock = Mock::start(vec![(404, r#"{"message":"Not Found"}"#.into())]).await;
    let gh = github(&mock, Some("tok123"));
    let err = GitHubClient::open_pull_request(&gh, "salim4n/ghost", "work", "main")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("not found on GitHub"), "{err}");
}

#[tokio::test]
async fn a_422_surfaces_githubs_own_message() {
    let mock = Mock::start(vec![(
        422,
        r#"{"message":"Validation Failed","errors":[{"message":"A pull request already exists"}]}"#
            .into(),
    )])
    .await;
    let gh = github(&mock, Some("tok123"));
    let err = GitHubClient::open_pull_request(&gh, "salim4n/mon-app", "work", "main")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("Validation Failed"), "{err}");
}

#[tokio::test]
async fn a_malformed_success_body_is_a_decode_error() {
    let mock = Mock::start(vec![(200, "this is not json".into())]).await;
    let gh = github(&mock, Some("tok123"));
    let err = GitHubClient::list_repos(&gh).await.unwrap_err();
    assert!(
        err.to_string().contains("unexpected GitHub response"),
        "{err}"
    );
}

#[tokio::test]
async fn a_missing_token_never_leaves_the_vault_boundary() {
    let mock = Mock::start(vec![]).await;
    let gh = github(&mock, None);
    let err = GitHubClient::list_repos(&gh).await.unwrap_err();
    assert!(err.to_string().contains("no GitHub token"), "{err}");
    assert!(
        mock.take_requests().is_empty(),
        "without a token, no request may go out"
    );
}

#[tokio::test]
async fn a_dead_host_is_a_network_error() {
    let gh = GitHub::new(
        GitHubConfig {
            api_base: "http://127.0.0.1:1".into(), // port 1: nothing listens
            ..GitHubConfig::default()
        },
        MemSecrets(HashMap::from([(DEFAULT_TOKEN_NAME.into(), "tok".into())])),
        GitHub::<MemSecrets>::default_http(),
    );
    let err = GitHubClient::list_repos(&gh).await.unwrap_err();
    assert!(
        err.to_string().contains("talking to GitHub failed"),
        "{err}"
    );
}
