#!/usr/bin/env python3
"""Opt-in, real-provider vertical canary for LaToile V1.

The canary deliberately lives outside Cargo's hermetic test graph. It creates
one private disposable GitHub repository, drives the public HTTP API, and
retains only bounded identifiers and delivery evidence. Provider output,
prompts, credentials, and hidden reasoning are never written to its artifact.
"""

from __future__ import annotations

import argparse
import contextlib
import datetime as dt
import json
import os
import pathlib
import secrets
import shutil
import signal
import socket
import sqlite3
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
from typing import Any


REPO_ROOT = pathlib.Path(__file__).resolve().parents[1]
DEFAULT_ARTIFACT_ROOT = REPO_ROOT / ".latoile-canary"
REPO_PREFIX = "latoile-v1-canary-"
ROLES = ("manager", "architect", "backend", "frontend", "reviewer")
TERMINAL_RUNS = {"finished", "error", "cancelled"}


class CanaryFailure(RuntimeError):
    """A controlled failure whose message is safe to show and persist."""


class ApiFailure(CanaryFailure):
    def __init__(self, method: str, path: str, status: int):
        self.status = status
        super().__init__(f"{method} {path} returned HTTP {status}")


def utc_now() -> str:
    return dt.datetime.now(dt.timezone.utc).isoformat().replace("+00:00", "Z")


def bounded(value: str, limit: int = 1000) -> str:
    value = " ".join(value.split())
    return value if len(value) <= limit else value[:limit] + "..."


def redact(value: str, secrets_to_hide: list[str]) -> str:
    """Redact known credentials and drop hidden-reasoning shaped lines."""
    clean_lines = []
    for line in value.splitlines() or [value]:
        lowered = line.lower()
        if "reasoning_content" in lowered or "thought_chunk" in lowered:
            continue
        for secret in secrets_to_hide:
            if secret:
                line = line.replace(secret, "[redacted]")
        clean_lines.append(line)
    return bounded("\n".join(clean_lines))


def atomic_json(path: pathlib.Path, payload: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(".tmp")
    temporary.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n")
    temporary.replace(path)


def require_binary(name: str) -> str:
    found = shutil.which(name)
    if not found:
        raise CanaryFailure(
            f"missing required binary '{name}'; install it and make it available on PATH"
        )
    return found


def checked(
    args: list[str],
    *,
    cwd: pathlib.Path | None = None,
    input_text: str | None = None,
    label: str | None = None,
) -> str:
    completed = subprocess.run(
        args,
        cwd=cwd,
        input=input_text,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
        env={**os.environ, "GIT_TERMINAL_PROMPT": "0"},
    )
    if completed.returncode != 0:
        raise CanaryFailure(
            f"{label or pathlib.Path(args[0]).name} exited with status {completed.returncode}"
        )
    return completed.stdout.strip()


def free_port() -> int:
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        return int(listener.getsockname()[1])


class Api:
    def __init__(self, base: str, token: str, agent_timeout: int):
        self.base = base.rstrip("/")
        self.token = token
        self.agent_timeout = agent_timeout

    def request(
        self, method: str, path: str, payload: dict[str, Any] | None = None
    ) -> Any:
        data = None
        headers = {"Authorization": f"Bearer {self.token}"}
        if payload is not None:
            data = json.dumps(payload).encode()
            headers["Content-Type"] = "application/json"
        request = urllib.request.Request(
            self.base + path, data=data, headers=headers, method=method
        )
        try:
            timeout = (
                self.agent_timeout
                if method == "POST" and path.endswith("/messages")
                else 60
            )
            with urllib.request.urlopen(request, timeout=timeout) as response:
                raw = response.read(2 * 1024 * 1024)
        except urllib.error.HTTPError as error:
            error.read(64 * 1024)  # drain, but never surface agent-controlled text
            raise ApiFailure(method, path, error.code) from None
        except (urllib.error.URLError, TimeoutError) as error:
            raise CanaryFailure(f"{method} {path} could not reach LaToile") from error
        return json.loads(raw) if raw else None

    def get(self, path: str) -> Any:
        return self.request("GET", path)

    def post(self, path: str, payload: dict[str, Any] | None = None) -> Any:
        return self.request("POST", path, payload)

    def put(self, path: str, payload: dict[str, Any]) -> Any:
        return self.request("PUT", path, payload)

    def text(self, path: str) -> str:
        request = urllib.request.Request(
            self.base + path,
            headers={"Authorization": f"Bearer {self.token}"},
            method="GET",
        )
        try:
            with urllib.request.urlopen(request, timeout=10) as response:
                return response.read(2 * 1024 * 1024).decode("utf-8", errors="replace")
        except urllib.error.HTTPError as error:
            error.read(64 * 1024)
            raise ApiFailure("GET", path, error.code) from None
        except (urllib.error.URLError, TimeoutError) as error:
            raise CanaryFailure(f"GET {path} could not reach LaToile") from error


class Canary:
    def __init__(self, args: argparse.Namespace):
        self.args = args
        stamp = dt.datetime.now(dt.timezone.utc).strftime("%Y%m%d-%H%M%S")
        self.run_id = f"{stamp}-{secrets.token_hex(3)}"
        self.artifact_dir = pathlib.Path(args.artifact_root).resolve() / self.run_id
        self.evidence_path = self.artifact_dir / "evidence.json"
        self.temporary = tempfile.TemporaryDirectory(prefix="latoile-v1-canary-")
        self.temp_root = pathlib.Path(self.temporary.name)
        self.home = self.temp_root / "home"
        self.seed = self.temp_root / "seed"
        self.server: subprocess.Popen[str] | None = None
        self.server_log_path = self.temp_root / "server.log"
        self.server_log: Any | None = None
        self.started_at = utc_now()
        self.first_broken_seam: str | None = None
        self.known_secrets: list[str] = []
        self.data: dict[str, Any] = {
            "schema_version": 1,
            "canary_run_id": self.run_id,
            "provider": args.provider,
            "started_at": self.started_at,
            "status": "running",
        }

    def stage(self, name: str) -> None:
        self.first_broken_seam = name
        print(f"[canary] {name}", flush=True)

    def preflight(self) -> tuple[str, str, str]:
        self.stage("preflight")
        binaries = ["git", "gh", "node"]
        provider_cli = self.args.provider
        adapter = "claude-agent-acp" if provider_cli == "claude" else "codex-acp"
        for binary in [*binaries, provider_cli, adapter]:
            require_binary(binary)
        cargo = os.environ.get("CARGO") or require_binary("cargo")

        checked(["gh", "auth", "status"], label="GitHub authentication check")
        status = (
            ["claude", "auth", "status"]
            if provider_cli == "claude"
            else ["codex", "login", "status"]
        )
        try:
            checked(status, label=f"{provider_cli} authentication check")
        except CanaryFailure as error:
            raise CanaryFailure(
                f"{provider_cli} is not authenticated; log in with its native CLI first"
            ) from error

        github_token = checked(["gh", "auth", "token"], label="GitHub token lookup")
        if not github_token:
            raise CanaryFailure("GitHub CLI returned no token; run 'gh auth login'")
        self.known_secrets.append(github_token)
        owner = self.args.repo_owner or checked(
            ["gh", "api", "user", "--jq", ".login"], label="GitHub owner lookup"
        )
        if not owner or "/" in owner:
            raise CanaryFailure("GitHub repository owner could not be resolved safely")
        return cargo, github_token, owner

    def build(self, cargo: str) -> pathlib.Path:
        self.stage("build server")
        checked([cargo, "build", "--quiet", "-p", "latoile-cli"], cwd=REPO_ROOT)
        binary = REPO_ROOT / "target" / "debug" / "latoile"
        if not binary.is_file():
            raise CanaryFailure("the latoile CLI binary was not produced")
        return binary

    def provision_repository(self, owner: str) -> str:
        self.stage("provision disposable repository")
        repo = f"{owner}/{REPO_PREFIX}{self.run_id.lower()}"
        # Record the exact bounded cleanup target even if `gh repo create`
        # fails after GitHub accepted the repository but before it exits.
        self.data["repository"] = repo
        self.seed.mkdir(parents=True)
        (self.seed / "design").mkdir()
        (self.seed / "package.json").write_text(
            '{"private":true,"scripts":{"dev":"node server.mjs"}}\n'
        )
        (self.seed / "server.mjs").write_text(
            """import http from 'node:http';
import { readFile } from 'node:fs/promises';
const index = process.argv.indexOf('--port');
const port = Number(index >= 0 ? process.argv[index + 1] : process.env.PORT || 4100);
http.createServer(async (_request, response) => {
  response.setHeader('content-type', 'text/html; charset=utf-8');
  response.end(await readFile(new URL('./index.html', import.meta.url)));
}).listen(port, '127.0.0.1');
"""
        )
        (self.seed / "index.html").write_text(
            "<!doctype html><html><body><main><h1>Canary pending</h1></main></body></html>\n"
        )
        (self.seed / "design" / "canary.md").write_text(
            "# V1 canary visual contract\n\nThe page must render the exact heading `LaToile Canary Ready`.\n"
        )
        (self.seed / "README.md").write_text(
            "# Disposable LaToile V1 canary fixture\n\nSafe to delete after evidence review.\n"
        )
        checked(["git", "init", "-b", "main"], cwd=self.seed)
        checked(["git", "config", "user.name", "LaToile Canary"], cwd=self.seed)
        checked(
            ["git", "config", "user.email", "canary@localhost.invalid"], cwd=self.seed
        )
        checked(["git", "add", "."], cwd=self.seed)
        checked(["git", "commit", "-m", "chore: seed V1 canary fixture"], cwd=self.seed)
        checked(
            [
                "gh",
                "repo",
                "create",
                repo,
                "--private",
                "--source",
                str(self.seed),
                "--remote",
                "origin",
                "--push",
                "--description",
                "Disposable LaToile V1 canary fixture",
            ],
            label="GitHub repository creation",
        )
        return repo

    def start_server(self, binary: pathlib.Path, github_token: str) -> Api:
        self.stage("start isolated LaToile server")
        self.home.mkdir(parents=True)
        stored = subprocess.run(
            [str(binary), "--home", str(self.home), "secret", "set", "github_token"],
            input=github_token + "\n",
            text=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=False,
        )
        if stored.returncode != 0:
            raise CanaryFailure("the GitHub token could not be stored in the isolated vault")

        port = free_port()
        api_token = secrets.token_urlsafe(32)
        self.known_secrets.append(api_token)
        self.server_log = self.server_log_path.open("wb")
        self.server = subprocess.Popen(
            [
                str(binary),
                "--home",
                str(self.home),
                "serve",
                "--port",
                str(port),
                "--workspace",
                str(self.home / "workspace"),
                "--skills-dir",
                str(REPO_ROOT / "skills"),
                "--token",
                api_token,
            ],
            stdout=self.server_log,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )
        base = f"http://127.0.0.1:{port}"
        deadline = time.monotonic() + 45
        while time.monotonic() < deadline:
            if self.server.poll() is not None:
                raise CanaryFailure("the isolated LaToile server exited during startup")
            try:
                with urllib.request.urlopen(base + "/api/health", timeout=1) as response:
                    if response.status == 200:
                        return Api(base, api_token, self.args.timeout)
            except (urllib.error.URLError, TimeoutError):
                time.sleep(0.25)
        raise CanaryFailure("the isolated LaToile server did not become healthy in 45 seconds")

    def create_project(self, api: Api, repo: str) -> dict[str, Any]:
        self.stage("create project checkout")
        routing = {role: self.args.provider for role in ROLES}
        api.put("/api/settings/routing", routing)
        branch = f"canary/{self.run_id.lower()}"
        project = api.post(
            "/api/projects",
            {
                "name": f"V1 Canary {self.run_id}",
                "slug": f"v1-canary-{self.run_id.lower()}",
                "github_repo": repo,
                "work_branch": branch,
                "dev_command": "node server.mjs --port $PORT",
            },
        )
        for key in ("id", "local_path", "work_branch", "default_branch"):
            if not project.get(key):
                raise CanaryFailure(f"project response is missing {key}")
        checkout = pathlib.Path(project["local_path"])
        checked(["git", "config", "user.name", "LaToile Canary"], cwd=checkout)
        checked(
            ["git", "config", "user.email", "canary@localhost.invalid"], cwd=checkout
        )
        self.data.update(
            {
                "project_id": project["id"],
                "work_branch": project["work_branch"],
                "default_branch": project["default_branch"],
            }
        )
        return project

    def manager_and_spec(self, api: Api, project: dict[str, Any]) -> str:
        self.stage("Manager turn")
        response = api.post(
            f"/api/projects/{project['id']}/messages",
            {
                "content": (
                    "Canary V1 deterministe. Propose exactement une specification dans design/ "
                    "avec une action propose_spec. Ne cree et ne lance aucune tache dans ce tour."
                )
            },
        )
        reply = response.get("reply")
        if not reply or not reply.get("id"):
            raise CanaryFailure("the real Manager adapter returned no persisted reply")
        tasks = api.get(f"/api/projects/{project['id']}/tasks")
        if tasks:
            raise CanaryFailure("the Manager created an unexpected task during the spec-only turn")
        specs = api.get(f"/api/projects/{project['id']}/spec-versions")
        drafts = [spec for spec in specs if spec.get("status") == "draft"]
        if len(drafts) != 1 or drafts[0].get("design_dir") != "design/":
            raise CanaryFailure("the Manager did not produce the single expected draft spec")
        self.data["manager_message_id"] = reply["id"]

        self.stage("human spec approval")
        spec = api.post(f"/api/spec-versions/{drafts[0]['id']}/approve")
        if spec.get("status") != "approved":
            raise CanaryFailure("the draft spec was not approved")
        self.data["spec_version_id"] = spec["id"]
        return spec["id"]

    def dispatch_executor(self, api: Api, project: dict[str, Any]) -> tuple[str, str]:
        self.stage("executor dispatch")
        title = "Build the V1 canary page"
        api.post(
            f"/api/projects/{project['id']}/tasks",
            {
                "role_id": "frontend",
                "title": title,
                "description": "Implement the approved canary visual contract.",
                "prompt": (
                    "Read design/canary.md. Change index.html so its h1 is exactly "
                    "'LaToile Canary Ready'. Keep the no-dependency dev server working. "
                    "Commit every change with message 'feat: complete V1 canary'. Finish with "
                    "a clean worktree and a changed HEAD. Do not touch secrets or Docker."
                ),
            },
        )
        tasks = api.get(f"/api/projects/{project['id']}/tasks")
        selected = [task for task in tasks if task.get("title") == title]
        if len(tasks) != 1 or len(selected) != 1 or not selected[0].get("latest_run_id"):
            raise CanaryFailure("executor dispatch did not create exactly one running task")
        task, run_id = selected[0], selected[0]["latest_run_id"]
        self.data["task_id"] = task["id"]
        self.data["executor_run_id"] = run_id
        return task["id"], run_id

    def decide_permissions(self, api: Api) -> None:
        approvals = api.get("/api/approvals")
        for approval in approvals:
            if approval.get("kind") != "permission":
                continue
            if not self.args.approve_permissions:
                raise CanaryFailure(
                    "an ACP mutation permission is pending; rerun with --approve-permissions"
                )
            api.post(
                f"/api/approvals/{approval['id']}",
                {
                    "granted": True,
                    "comment": "Explicitly authorized by the opt-in V1 canary command.",
                },
            )

    def wait_for_executor(self, api: Api, run_id: str) -> dict[str, Any]:
        self.stage("executor run")
        deadline = time.monotonic() + self.args.timeout
        while time.monotonic() < deadline:
            self.decide_permissions(api)
            run = api.get(f"/api/runs/{run_id}")
            if run.get("status") in TERMINAL_RUNS:
                if run.get("status") != "finished":
                    raise CanaryFailure(f"executor ended with status {run.get('status')}")
                base, head = run.get("base_sha"), run.get("head_sha")
                if not base or not head or base == head or len(head) != 40:
                    raise CanaryFailure("executor produced no committed SHA change")
                self.data["executor_base_sha"] = base
                self.data["executor_head_sha"] = head
                return run
            time.sleep(1)
        raise CanaryFailure("executor run exceeded the canary timeout")

    def wait_for_review(self, api: Api, project_id: str) -> tuple[str, str]:
        self.stage("Reviewer result")
        deadline = time.monotonic() + self.args.timeout
        while time.monotonic() < deadline:
            self.decide_permissions(api)
            approvals = api.get("/api/approvals")
            review = next((a for a in approvals if a.get("kind") == "review"), None)
            if review:
                try:
                    payload = json.loads(review.get("payload") or "{}")
                except json.JSONDecodeError as error:
                    raise CanaryFailure("Reviewer approval payload is not valid JSON") from error
                verdict = payload.get("verdict")
                if verdict not in {"approve", "approve_with_reservations"}:
                    raise CanaryFailure(f"Reviewer returned non-deliverable verdict {verdict!r}")
                reviewer_run = api.get(f"/api/runs/{review['run_id']}")
                if reviewer_run.get("status") != "finished":
                    raise CanaryFailure("Reviewer approval appeared before a finished reviewer run")
                self.data["reviewer_run_id"] = review["run_id"]
                self.data["review_verdict"] = verdict

                self.stage("human review approval")
                decided = api.post(
                    f"/api/approvals/{review['id']}",
                    {"granted": True, "comment": "V1 canary review explicitly approved."},
                )
                if decided.get("status") != "granted":
                    raise CanaryFailure("human review approval was not persisted")
                return review["run_id"], verdict
            tasks = api.get(f"/api/projects/{project_id}/tasks")
            if tasks:
                latest = tasks[0].get("latest_run_id")
                if latest and latest != self.data.get("executor_run_id"):
                    reviewer_run = api.get(f"/api/runs/{latest}")
                    if reviewer_run.get("status") in {"error", "cancelled"}:
                        raise CanaryFailure(
                            f"Reviewer ended with status {reviewer_run.get('status')}"
                        )
            time.sleep(1)
        raise CanaryFailure("Reviewer result exceeded the canary timeout")

    def prove_preview(self, api: Api, project_id: str) -> None:
        self.stage("live preview")
        try:
            preview = api.get(f"/api/projects/{project_id}/preview")
        except ApiFailure as error:
            if error.status != 404:
                raise
            preview = api.post(f"/api/projects/{project_id}/preview")
        if (
            preview.get("status") != "ready"
            or not preview.get("alive")
            or not preview.get("port")
        ):
            raise CanaryFailure("the project preview is not ready")
        rendered = api.text(f"/api/projects/{project_id}/preview/")
        if "LaToile Canary Ready" not in rendered:
            raise CanaryFailure("the live preview does not render the approved canary heading")
        self.data["preview_status"] = "ready"

    def deliver(self, api: Api, project: dict[str, Any], repo: str) -> None:
        self.stage("Pull Request delivery")
        tasks = api.get(f"/api/projects/{project['id']}/tasks")
        if len(tasks) != 1 or tasks[0].get("status") != "done":
            raise CanaryFailure("the reviewed task is not done before delivery")
        delivery = api.post(f"/api/projects/{project['id']}/delivery")
        local_sha, remote_sha = delivery.get("local_sha"), delivery.get("remote_sha")
        pull_request_url = delivery.get("pull_request_url")
        if (
            delivery.get("status") != "pull_request_open"
            or not local_sha
            or local_sha != remote_sha
            or not str(pull_request_url).startswith(f"https://github.com/{repo}/pull/")
        ):
            raise CanaryFailure("delivery did not return a verified SHA and Pull Request URL")
        remote_ref = checked(
            [
                "gh",
                "api",
                f"repos/{repo}/git/ref/heads/{project['work_branch']}",
                "--jq",
                ".object.sha",
            ],
            label="remote SHA verification",
        )
        if remote_ref != local_sha:
            raise CanaryFailure("GitHub remote ref does not equal the delivered local SHA")
        self.data.update(
            {
                "delivery_status": delivery["status"],
                "local_sha": local_sha,
                "remote_sha": remote_sha,
                "pull_request_url": pull_request_url,
            }
        )

    def event_cursor(self) -> int:
        project_id = self.data.get("project_id")
        database = self.home / "latoile.db"
        if not project_id or not database.exists():
            return 0
        with sqlite3.connect(database) as connection:
            row = connection.execute(
                "SELECT COALESCE(MAX(seq), 0) FROM event WHERE project_id = ?",
                (project_id,),
            ).fetchone()
        return int(row[0]) if row else 0

    def write_evidence(self, status: str, error: Exception | None = None) -> None:
        self.data["status"] = status
        self.data["finished_at"] = utc_now()
        self.data["event_cursor"] = self.event_cursor()
        if error is not None:
            self.data["first_broken_seam"] = self.first_broken_seam or "bootstrap"
            self.data["failure"] = redact(str(error), self.known_secrets)
            diagnostic = self.safe_server_diagnostic()
            if diagnostic:
                self.data["adapter_diagnostic"] = diagnostic
        else:
            self.data["first_broken_seam"] = None
        repo = self.data.get("repository")
        if repo:
            self.data["cleanup"] = {
                "remote_retained": True,
                "command": (
                    f"python3 scripts/v1_canary.py cleanup --evidence "
                    f"{self.evidence_path} --confirm {repo}"
                ),
            }
        atomic_json(self.evidence_path, self.data)

    def safe_server_diagnostic(self) -> str | None:
        """Return one filtered adapter lifecycle error, never provider content."""
        if not self.server_log_path.exists():
            return None
        with contextlib.suppress(OSError):
            lines = self.server_log_path.read_text(errors="replace").splitlines()
            for line in reversed(lines):
                lowered = line.lower()
                if not any(
                    marker in lowered
                    for marker in (
                        "request failed inside an adapter",
                        "reviewer dispatch failed",
                        "supervision tick failed",
                    )
                ):
                    continue
                if any(
                    forbidden in lowered
                    for forbidden in (
                        "reasoning",
                        "thought",
                        "authorization",
                        "bearer",
                        "github_token",
                        "api_key",
                    )
                ):
                    return "adapter lifecycle failed; detail withheld by the canary redaction policy"
                return redact(line, self.known_secrets)
        return None

    def close(self) -> None:
        if self.server and self.server.poll() is None:
            with contextlib.suppress(ProcessLookupError):
                os.killpg(self.server.pid, signal.SIGTERM)
            try:
                self.server.wait(timeout=10)
            except subprocess.TimeoutExpired:
                with contextlib.suppress(ProcessLookupError):
                    os.killpg(self.server.pid, signal.SIGKILL)
                self.server.wait(timeout=5)
        if self.server_log:
            self.server_log.close()
        self.temporary.cleanup()

    def run(self) -> None:
        cargo, github_token, owner = self.preflight()
        binary = self.build(cargo)
        repo = self.provision_repository(owner)
        api = self.start_server(binary, github_token)
        project = self.create_project(api, repo)
        self.manager_and_spec(api, project)
        _, executor_run = self.dispatch_executor(api, project)
        self.wait_for_executor(api, executor_run)
        self.wait_for_review(api, project["id"])
        self.prove_preview(api, project["id"])
        self.deliver(api, project, repo)


def cleanup(args: argparse.Namespace) -> int:
    evidence_path = pathlib.Path(args.evidence).resolve()
    try:
        evidence = json.loads(evidence_path.read_text())
    except (OSError, json.JSONDecodeError) as error:
        print(f"cleanup refused: unreadable evidence: {bounded(str(error))}", file=sys.stderr)
        return 2
    repo = evidence.get("repository")
    name = str(repo).split("/", 1)[-1] if repo else ""
    if (
        not repo
        or repo != args.confirm
        or not name.startswith(REPO_PREFIX)
        or evidence.get("schema_version") != 1
    ):
        print(
            "cleanup refused: --confirm must exactly match the disposable repository in evidence",
            file=sys.stderr,
        )
        return 2
    try:
        action = "delete" if args.delete else "archive"
        checked(
            ["gh", "repo", action, repo, "--yes"],
            label=f"GitHub repository {action}",
        )
    except CanaryFailure as error:
        print(f"cleanup failed: {error}", file=sys.stderr)
        return 1
    evidence.setdefault("cleanup", {})
    if args.delete:
        evidence["cleanup"].update(
            {"remote_retained": False, "remote_deleted_at": utc_now()}
        )
        outcome = "deleted"
    else:
        evidence["cleanup"].update(
            {
                "remote_retained": True,
                "remote_archived": True,
                "remote_archived_at": utc_now(),
            }
        )
        outcome = "archived"
    atomic_json(evidence_path, evidence)
    print(f"{outcome} disposable repository {repo}; evidence retained at {evidence_path}")
    return 0


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(description=__doc__)
    commands = root.add_subparsers(dest="command", required=True)
    run = commands.add_parser("run", help="run the opt-in real-provider canary")
    run.add_argument("--provider", choices=("claude", "codex"), required=True)
    run.add_argument(
        "--approve-permissions",
        action="store_true",
        help="explicitly grant sanitized ACP mutation requests inside the disposable checkout",
    )
    run.add_argument("--repo-owner", help="GitHub owner; defaults to the authenticated user")
    run.add_argument("--timeout", type=int, default=20 * 60, help="seconds per agent phase")
    run.add_argument("--artifact-root", default=str(DEFAULT_ARTIFACT_ROOT))
    clean = commands.add_parser("cleanup", help="archive or delete a disposable repository")
    clean.add_argument("--evidence", required=True)
    clean.add_argument("--confirm", required=True)
    clean.add_argument(
        "--delete",
        action="store_true",
        help="delete instead of archiving; requires a GitHub token with delete_repo",
    )
    return root


def main(argv: list[str] | None = None) -> int:
    args = parser().parse_args(argv)
    if args.command == "cleanup":
        return cleanup(args)
    canary = Canary(args)
    succeeded = False
    try:
        canary.run()
        canary.write_evidence("success")
        succeeded = True
    except (CanaryFailure, OSError, sqlite3.Error) as error:
        canary.write_evidence("failed", error)
        print(
            f"[canary] FAILED at {canary.first_broken_seam or 'bootstrap'}: "
            f"{redact(str(error), canary.known_secrets)}",
            file=sys.stderr,
        )
        print(f"[canary] evidence: {canary.evidence_path}", file=sys.stderr)
        return 1
    finally:
        canary.close()
    if not succeeded:
        return 1
    print(f"[canary] PASS: {canary.data['pull_request_url']}")
    print(f"[canary] verified SHA: {canary.data['remote_sha']}")
    print(f"[canary] evidence: {canary.evidence_path}")
    print(f"[canary] cleanup: {canary.data['cleanup']['command']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
