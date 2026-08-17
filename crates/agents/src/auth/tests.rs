//! Auth-manager tests: login commands are faked with `sh -c` one-liners.
//! Claude: prints an ANSI-wrapped URL, reads a line, exits 0 iff "good".
//! Codex: prints a URL and a device code, polls (sleeps), exits on its own.
//! No real `claude` or `codex` binary is ever touched.

use super::*;

/// Claude, succeeding on code "good".
const FAKE_OK: &str = "printf '\\033[32mhttps://claude.com/oauth/authorize?client=abc\\033[0m.\\n'; read line; [ \"$line\" = good ]";
/// Claude, always failing.
const FAKE_KO: &str =
    "printf 'https://claude.com/oauth/authorize?client=abc\\n'; read line; exit 1";
/// Prints the URL, then hangs.
const FAKE_HANG: &str = "printf 'https://claude.com/oauth/authorize?client=abc\\n'; sleep 60";
/// Codex: URL + device code, then exits 0 (the user confirmed in time).
const FAKE_CODEX_OK: &str =
    "printf 'Go to https://auth.openai.com/codex/device and enter ABCD-EFGH\\n'; sleep 0.2";
/// Codex: same, then the poll gives up (exit 1).
const FAKE_CODEX_KO: &str =
    "printf 'https://auth.openai.com/codex/device\\nABCD-EFGH\\n'; sleep 0.2; exit 1";

fn manager(script: &str) -> AgentAuthManager {
    AgentAuthManager::new(DEFAULT_TTL).with_command(
        AuthProvider::Claude,
        AgentCommand::new("sh").args(["-c", script]),
    )
}

fn codex_manager(script: &str) -> AgentAuthManager {
    AgentAuthManager::new(DEFAULT_TTL).with_command(
        AuthProvider::Codex,
        AgentCommand::new("sh").args(["-c", script]),
    )
}

async fn wait_for(mgr: &AgentAuthManager, id: &str, want: AuthStatus) -> AuthSessionView {
    for _ in 0..200 {
        let view = mgr.status(id).unwrap();
        if view.status == want {
            return view;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the session never reached {want:?}");
}

async fn wait_terminal(mgr: &AgentAuthManager, id: &str) -> AuthSessionView {
    for _ in 0..200 {
        let view = mgr.status(id).unwrap();
        if view.status.is_terminal() {
            return view;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the session never finished");
}

// ── Claude (unchanged flow) ──────────────────────────────────────────────────

#[tokio::test]
async fn the_happy_path_ends_authenticated() {
    let mgr = manager(FAKE_OK);
    let session = mgr.start(AuthProvider::Claude).await.unwrap();
    assert_eq!(session.status, AuthStatus::Starting);
    assert!(session.input_required);

    let waiting = wait_for(&mgr, &session.id, AuthStatus::WaitingForInput).await;
    assert_eq!(
        waiting.url.as_deref(),
        Some("https://claude.com/oauth/authorize?client=abc"),
        "ANSI stripped, trailing punctuation trimmed"
    );

    let validating = mgr.submit_code(&session.id, "good").await.unwrap();
    assert_eq!(validating.status, AuthStatus::Validating);

    let done = wait_terminal(&mgr, &session.id).await;
    assert_eq!(done.status, AuthStatus::Authenticated);
    assert!(done.error.is_none());
}

#[tokio::test]
async fn a_wrong_code_fails_the_session() {
    let mgr = manager(FAKE_KO);
    let session = mgr.start(AuthProvider::Claude).await.unwrap();
    wait_for(&mgr, &session.id, AuthStatus::WaitingForInput).await;
    mgr.submit_code(&session.id, "bad").await.unwrap();

    let done = wait_terminal(&mgr, &session.id).await;
    assert_eq!(done.status, AuthStatus::Failed);
    assert!(done.error.is_some());
}

#[tokio::test]
async fn a_challenge_past_its_ttl_expires_and_kills_the_child() {
    let mgr = AgentAuthManager::new(Duration::from_millis(300)).with_command(
        AuthProvider::Claude,
        AgentCommand::new("sh").args(["-c", FAKE_HANG]),
    );
    let session = mgr.start(AuthProvider::Claude).await.unwrap();
    wait_for(&mgr, &session.id, AuthStatus::WaitingForInput).await;

    let view = wait_for(&mgr, &session.id, AuthStatus::Expired).await;
    assert_eq!(view.status, AuthStatus::Expired);
    assert!(mgr.submit_code(&session.id, "good").await.is_err());
}

#[tokio::test]
async fn a_code_before_the_url_is_refused() {
    let mgr = manager(FAKE_HANG);
    let session = mgr.start(AuthProvider::Claude).await.unwrap();
    assert!(matches!(
        mgr.submit_code(&session.id, "early").await,
        Err(AuthError::NotWaiting)
    ));
}

#[tokio::test]
async fn an_unknown_session_is_unknown() {
    let mgr = manager(FAKE_HANG);
    assert!(mgr.status("nope").is_none());
    assert!(matches!(
        mgr.submit_code("nope", "x").await,
        Err(AuthError::Unknown)
    ));
}

#[tokio::test]
async fn a_missing_binary_is_a_spawn_error() {
    let mgr = AgentAuthManager::new(DEFAULT_TTL).with_command(
        AuthProvider::Claude,
        AgentCommand::new("definitely-not-a-real-binary-latoile"),
    );
    assert!(matches!(
        mgr.start(AuthProvider::Claude).await,
        Err(AuthError::Spawn(_))
    ));
}

// ── Codex (device flow — no stdin) ───────────────────────────────────────────

#[tokio::test]
async fn codex_waits_with_url_and_device_code_then_authenticates() {
    let mgr = codex_manager(FAKE_CODEX_OK);
    let session = mgr.start(AuthProvider::Codex).await.unwrap();
    assert!(!session.input_required);

    let waiting = wait_for(&mgr, &session.id, AuthStatus::WaitingForInput).await;
    assert_eq!(
        waiting.url.as_deref(),
        Some("https://auth.openai.com/codex/device")
    );
    assert_eq!(waiting.user_code.as_deref(), Some("ABCD-EFGH"));

    let done = wait_terminal(&mgr, &session.id).await;
    assert_eq!(done.status, AuthStatus::Authenticated);
}

#[tokio::test]
async fn codex_failure_is_failed() {
    let mgr = codex_manager(FAKE_CODEX_KO);
    let session = mgr.start(AuthProvider::Codex).await.unwrap();
    let done = wait_terminal(&mgr, &session.id).await;
    assert_eq!(done.status, AuthStatus::Failed);
}

#[tokio::test]
async fn codex_refuses_a_code_it_never_asked_for() {
    let mgr = codex_manager(FAKE_CODEX_OK);
    let session = mgr.start(AuthProvider::Codex).await.unwrap();
    wait_for(&mgr, &session.id, AuthStatus::WaitingForInput).await;
    assert!(matches!(
        mgr.submit_code(&session.id, "ABCD-EFGH").await,
        Err(AuthError::InputNotRequired)
    ));
}

#[cfg(unix)]
#[tokio::test]
async fn expiry_kills_the_whole_process_tree() {
    // The fake "CLI" spawns a grandchild that writes its pid and sleeps —
    // the codex wrapper → vendored binary shape.
    let dir = tempfile::tempdir().unwrap();
    let pidfile = dir.path().join("grandchild.pid");
    let script = format!(
        "printf 'https://auth.openai.com/codex/device\\nABCD-EFGH\\n'; sh -c 'echo $$ > {}; sleep 60' & wait",
        pidfile.display()
    );
    let mgr = AgentAuthManager::new(Duration::from_millis(300)).with_command(
        AuthProvider::Codex,
        AgentCommand::new("sh").args(["-c", &script]),
    );

    let session = mgr.start(AuthProvider::Codex).await.unwrap();
    wait_for(&mgr, &session.id, AuthStatus::WaitingForInput).await;

    let mut grandchild = None;
    for _ in 0..100 {
        if let Ok(pid) = std::fs::read_to_string(&pidfile) {
            grandchild = pid.trim().parse::<i32>().ok();
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let grandchild = grandchild.expect("the grandchild never wrote its pid");

    wait_for(&mgr, &session.id, AuthStatus::Expired).await;
    for _ in 0..100 {
        // kill(pid, 0): ESRCH means the process is gone.
        let alive = unsafe { libc::kill(grandchild, 0) } == 0;
        if !alive {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the grandchild survived the session's death");
}
