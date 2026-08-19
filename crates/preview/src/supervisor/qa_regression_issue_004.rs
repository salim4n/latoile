use super::*;
use latoile_core::ids::{PreviewId, ProjectId};
use latoile_core::ports::PreviewSupervisor;
use std::time::Duration;

#[tokio::test]
async fn automatic_command_is_redetected_after_greenfield_files_appear() {
    let checkout = tempfile::tempdir().unwrap();
    tokio::fs::write(
        checkout.path().join("package.json"),
        r#"{"scripts":{"dev":"node server.mjs"}}"#,
    )
    .await
    .unwrap();
    tokio::fs::write(
        checkout.path().join("server.mjs"),
        r#"import http from "node:http";
const args = process.argv.slice(2);
const at = args.indexOf("--port");
const port = Number(at >= 0 ? args[at + 1] : process.env.PORT);
http.createServer((_req, res) => res.end("ready")).listen(port, "127.0.0.1");"#,
    )
    .await
    .unwrap();

    let supervisor = Supervisor::new(SupervisorConfig {
        base_port: 25100,
        readiness: Duration::from_secs(15),
        ..SupervisorConfig::default()
    });
    let preview = Preview::new(
        PreviewId::new("qa-preview").unwrap(),
        ProjectId::new("qa-project").unwrap(),
        0,
        "work",
    );

    let (_pid, port) = supervisor
        .ensure(&preview, "", checkout.path().to_str().unwrap())
        .await
        .unwrap();
    assert!(port >= 25100);
    supervisor.stop(&preview).await.unwrap();
}
