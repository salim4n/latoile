#!/usr/bin/env python3
"""Opt-in, real-provider greenfield visual-contract canary for LaToile V1.

The canary deliberately lives outside Cargo's hermetic test graph. It creates
one private disposable GitHub repository with an empty initial tree, drives
the public HTTP API from Socratic discovery through a visual correction and
Pull Request, and retains only bounded identifiers and delivery evidence.
Provider output, prompts, credentials, and hidden reasoning are never written
to its artifact.
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
CAPTURE_BROWSER_CANDIDATES = (
    "google-chrome-stable",
    "google-chrome",
    "chromium",
    "chromium-browser",
)


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


def require_sha(value: Any, label: str, length: int = 64) -> str:
    text = str(value or "")
    if len(text) != length or any(character not in "0123456789abcdef" for character in text):
        raise CanaryFailure(f"{label} is not a lowercase {length}-character digest")
    return text


def require_capture_browser() -> str:
    configured = os.environ.get("LATOILE_CAPTURE_BROWSER")
    candidates = [pathlib.Path(configured)] if configured else []
    candidates.extend(
        pathlib.Path(path)
        for name in CAPTURE_BROWSER_CANDIDATES
        if (path := shutil.which(name))
    )
    candidates.extend(
        [
            pathlib.Path("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"),
            pathlib.Path("/Applications/Chromium.app/Contents/MacOS/Chromium"),
        ]
    )
    browser = next((candidate for candidate in candidates if candidate.is_file()), None)
    if browser is None:
        raise CanaryFailure(
            "missing Chrome/Chromium for real visual capture; install it or set "
            "LATOILE_CAPTURE_BROWSER"
        )
    return str(browser)


def one_visual_comparison(
    comparisons: Any, reviewed_run_id: str, expected_status: str
) -> dict[str, Any]:
    if not isinstance(comparisons, list) or len(comparisons) != 1:
        raise CanaryFailure("the canary requires exactly one trusted visual comparison")
    evidence = comparisons[0]
    if not isinstance(evidence, dict):
        raise CanaryFailure("visual comparison response is malformed")
    if evidence.get("run_id") != reviewed_run_id:
        raise CanaryFailure("visual evidence is bound to the wrong executor run")
    if evidence.get("status") != expected_status:
        raise CanaryFailure(
            f"visual evidence is {evidence.get('status')!r}, expected {expected_status!r}"
        )
    require_sha(evidence.get("baseline_png_digest"), "visual baseline digest")
    require_sha(evidence.get("manifest_digest"), "visual manifest digest")
    require_sha(evidence.get("render_png_digest"), "visual render digest")
    require_sha(evidence.get("pixel_diff_digest"), "visual pixel diff digest")
    require_sha(evidence.get("heatmap_png_digest"), "visual heatmap digest")
    return evidence


def prove_replacement_evidence(
    original: dict[str, Any], corrected: dict[str, Any]
) -> None:
    if original.get("id") == corrected.get("id"):
        raise CanaryFailure("corrective evidence reused the original evidence id")
    if original.get("run_id") == corrected.get("run_id"):
        raise CanaryFailure("corrective evidence reused the original executor run")
    if original.get("comparison_id") != corrected.get("comparison_id"):
        raise CanaryFailure("corrective evidence changed the approved scenario")
    if original.get("baseline_png_digest") != corrected.get("baseline_png_digest"):
        raise CanaryFailure("corrective evidence changed the immutable baseline")


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
        self.permission_decisions = 0
        self.data: dict[str, Any] = {
            "schema_version": 2,
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
        require_capture_browser()
        cargo = os.environ.get("CARGO") or require_binary("cargo")
        if checked(["git", "status", "--porcelain"], cwd=REPO_ROOT):
            raise CanaryFailure("the real-provider canary requires a clean committed LaToile tree")
        self.data["latoile_commit"] = require_sha(
            checked(["git", "rev-parse", "HEAD"], cwd=REPO_ROOT),
            "LaToile commit",
            40,
        )

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
        self.stage("provision empty disposable repository")
        repo = f"{owner}/{REPO_PREFIX}{self.run_id.lower()}"
        # Record the exact bounded cleanup target even if `gh repo create`
        # fails after GitHub accepted the repository but before it exits.
        self.data["repository"] = repo
        self.seed.mkdir(parents=True)
        checked(["git", "init", "-b", "main"], cwd=self.seed)
        checked(["git", "config", "user.name", "LaToile Canary"], cwd=self.seed)
        checked(
            ["git", "config", "user.email", "canary@localhost.invalid"], cwd=self.seed
        )
        checked(
            ["git", "commit", "--allow-empty", "-m", "chore: initialize empty canary"],
            cwd=self.seed,
        )
        checked(
            [
                "gh",
                "repo",
                "create",
                repo,
                "--private",
                "--description",
                "Disposable LaToile greenfield visual-contract canary",
            ],
            label="GitHub repository creation",
        )
        checked(
            ["git", "remote", "add", "origin", f"https://github.com/{repo}.git"],
            cwd=self.seed,
        )
        checked(["git", "push", "-u", "origin", "main"], cwd=self.seed)
        tracked = checked(["git", "ls-tree", "-r", "--name-only", "HEAD"], cwd=self.seed)
        if tracked:
            raise CanaryFailure("the disposable repository initial tree is not empty")
        self.data["empty_initial_tree"] = True
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

    def manager_and_spec(
        self, api: Api, project: dict[str, Any]
    ) -> tuple[dict[str, Any], dict[str, Any]]:
        self.stage("Socratic Architect discovery")
        brief = (
            "Projet greenfield de canary: concevoir une mini application web mono-ecran, "
            "sans compte ni integration externe. Elle affiche un statut de livraison avec le "
            "titre visible exact 'Contrat visuel LaToile'. Exiger exactement un scenario P0 "
            "deterministe: route /, etat success, locale fr-FR, theme light, viewport 390x844 "
            "a echelle 1, donnees synthetiques, sans masque. Le mockup doit etre autonome, "
            "accessible et reproductible; la future implementation sera un serveur Node sans "
            "dependance. Challenge ces decisions avec le workflow Socratique complet avant de "
            "produire architecture et maquette."
        )
        response = api.post(
            f"/api/projects/{project['id']}/messages",
            {"content": brief, "intent": "architecture_brief"},
        )
        reply = response.get("reply")
        if not reply or not reply.get("id"):
            raise CanaryFailure("the real Architect adapter returned no persisted first turn")
        self.data["architect_first_message_id"] = reply["id"]

        for _ in range(12):
            architecture = api.get(f"/api/projects/{project['id']}/architecture")
            status = architecture.get("status")
            if status in {"failed", "cancelled"}:
                raise CanaryFailure(f"Architect discovery ended with status {status!r}")
            if architecture.get("package_status") == "draft_ready":
                break
            if status != "awaiting_answer":
                raise CanaryFailure(
                    f"Architect discovery stopped in unexpected status {status!r}"
                )
            questions = architecture.get("questions") or []
            open_questions = [
                question for question in questions if question.get("status") == "open"
            ]
            if len(open_questions) != 1 or not open_questions[0].get("id"):
                raise CanaryFailure("Architect did not expose exactly one durable open question")
            answer = (
                "Decision proprietaire: conserver le perimetre greenfield minimal et "
                "deterministe du brief. Un seul ecran et un seul scenario P0 success, route /, "
                "fr-FR, light, 390x844@1000, donnees synthetiques, aucun masque ni integration. "
                "Le titre visible exact est 'Contrat visuel LaToile'. Pour le point demande, "
                "retenir l'option la plus simple, accessible, testable et sans dependance; "
                "documenter le compromis puis poursuivre."
            )
            answered = api.post(
                f"/api/projects/{project['id']}/messages", {"content": answer}
            )
            if not answered.get("reply", {}).get("id"):
                raise CanaryFailure("Architect returned no persisted answer turn")
        else:
            raise CanaryFailure("Architect discovery exceeded 12 owner decisions")

        architecture = api.get(f"/api/projects/{project['id']}/architecture")
        questions = architecture.get("questions") or []
        if (
            architecture.get("status") != "ready_to_draft"
            or architecture.get("phase") != "ready_to_draft"
            or architecture.get("package_status") != "draft_ready"
            or architecture.get("skill_name") != "app-architect-brainstorm"
            or architecture.get("operating_mode") != "greenfield"
            or not questions
            or any(question.get("status") != "answered" for question in questions)
        ):
            raise CanaryFailure("Architect session lacks complete greenfield provenance")
        skill_digest = require_sha(
            architecture.get("skill_digest"), "Architect skill digest"
        )
        package = architecture.get("package") or {}
        package_digest = require_sha(package.get("package_digest"), "package digest")
        manifest_digest = require_sha(package.get("manifest_digest"), "manifest digest")
        package_commit = require_sha(package.get("head_sha"), "package commit", 40)
        package_tree = require_sha(package.get("tree_sha"), "package tree", 40)

        specs = api.get(f"/api/projects/{project['id']}/spec-versions")
        drafts = [spec for spec in specs if spec.get("status") == "draft"]
        if len(drafts) != 1:
            raise CanaryFailure("Architect did not produce exactly one draft spec")
        draft = drafts[0]
        if (
            draft.get("architecture_session_id") != architecture.get("id")
            or draft.get("skill_digest") != skill_digest
            or draft.get("package_digest") != package_digest
            or draft.get("manifest_digest") != manifest_digest
            or draft.get("package_commit_sha") != package_commit
            or draft.get("package_tree_sha") != package_tree
        ):
            raise CanaryFailure("draft spec does not pin the Architect package exactly")

        validation = api.get(f"/api/spec-versions/{draft['id']}/validation")
        scenarios = validation.get("scenarios") or []
        if (
            not validation.get("valid")
            or validation.get("file_count", 0) < 16
            or len(scenarios) != 1
            or validation.get("manifest_digest") != manifest_digest
            or validation.get("commit_sha") != package_commit
            or validation.get("tree_sha") != package_tree
        ):
            raise CanaryFailure("architecture package validation is incomplete or mismatched")
        scenario = scenarios[0]
        if (
            scenario.get("route") != "/"
            or scenario.get("locale") != "fr-FR"
            or scenario.get("theme") != "light"
            or scenario.get("viewport_width") != 390
            or scenario.get("viewport_height") != 844
            or scenario.get("device_scale_factor_milli") != 1000
            or scenario.get("allowed_masks")
        ):
            raise CanaryFailure("Architect changed the single deterministic visual scenario")
        mockup = scenario.get("mockup")
        if not isinstance(mockup, str) or not mockup.startswith("mockups/"):
            raise CanaryFailure("validated scenario has no bounded mockup path")
        encoded_mockup = urllib.parse.quote(mockup, safe="/")
        mockup_html = api.text(
            f"/api/spec-versions/{draft['id']}/artifacts/{encoded_mockup}"
        )
        if "Contrat visuel LaToile" not in mockup_html:
            raise CanaryFailure("the real Architect mockup omitted the owner-visible marker")

        checkout = pathlib.Path(project["local_path"])
        manifest_path = f"{draft['design_dir']}package-manifest.md"
        checked(["git", "cat-file", "-e", f"{package_commit}:{manifest_path}"], cwd=checkout)
        committed_tree = checked(["git", "rev-parse", f"{package_commit}^{{tree}}"], cwd=checkout)
        if committed_tree != package_tree:
            raise CanaryFailure("the immutable package tree does not match Git")

        self.data.update(
            {
                "architecture_session_id": architecture["id"],
                "architect_question_count": len(questions),
                "architect_skill_name": architecture["skill_name"],
                "architect_skill_digest": skill_digest,
                "architecture_operating_mode": architecture["operating_mode"],
                "architecture_file_count": validation["file_count"],
                "package_digest": package_digest,
                "manifest_digest": manifest_digest,
                "spec_commit_sha": package_commit,
                "spec_tree_sha": package_tree,
                "comparison_id": scenario["comparison_id"],
            }
        )

        self.stage("immutable spec and baseline approval")
        spec = api.post(f"/api/spec-versions/{draft['id']}/approve")
        if spec.get("status") != "approved" or spec.get("package_commit_sha") != package_commit:
            raise CanaryFailure("the immutable draft spec was not approved exactly")
        baselines = api.get(f"/api/spec-versions/{spec['id']}/baselines")
        if len(baselines) != 1 or baselines[0].get("status") != "ready":
            raise CanaryFailure("real Chromium baseline capture is not ready")
        baseline_digest = require_sha(
            baselines[0].get("png_digest"), "approved baseline PNG digest"
        )
        self.data.update(
            {
                "spec_version_id": spec["id"],
                "baseline_png_digest": baseline_digest,
                "baseline_browser_version": baselines[0].get("browser_version"),
            }
        )
        return spec, validation

    def dispatch_executor(self, api: Api, project: dict[str, Any]) -> tuple[str, str]:
        self.stage("deliberately regressed executor dispatch")
        title = "Build the greenfield visual canary page"
        api.post(
            f"/api/projects/{project['id']}/tasks",
            {
                "role_id": "frontend",
                "title": title,
                "description": (
                    "Implement the approved visual contract, including the temporary "
                    "canary-regression style described in the execution prompt."
                ),
                "prompt": (
                    "This repository has no application source. Read the approved architecture "
                    "package without changing any design/ file. It declares exactly one "
                    "self-contained mockups/*.html P0 page. Create package.json and server.mjs "
                    "for a no-dependency Node HTTP server. For every request, server.mjs must "
                    "read that approved mockup at runtime and return its HTML, but deliberately "
                    "inject exactly `<style id=\"canary-regression\">html { position: relative; "
                    "left: 16px !important; }</style>` immediately before </head>. Parse the "
                    "port from `--port` and listen only on 127.0.0.1. Keep the visible title "
                    "'Contrat visuel LaToile'. Commit every implementation change with message "
                    "'feat: introduce visual canary regression'. Finish with a clean worktree "
                    "and changed HEAD. Do not touch secrets, Docker or the approved design package."
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
            self.permission_decisions += 1

    def wait_for_executor(
        self, api: Api, run_id: str, evidence_prefix: str
    ) -> dict[str, Any]:
        self.stage(f"{evidence_prefix} executor run")
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
                self.data[f"{evidence_prefix}_executor_run_id"] = run_id
                self.data[f"{evidence_prefix}_executor_base_sha"] = base
                self.data[f"{evidence_prefix}_executor_head_sha"] = head
                return run
            time.sleep(1)
        raise CanaryFailure("executor run exceeded the canary timeout")

    def prove_reviewer_before_human(self, reviewer_run_id: str, prefix: str) -> None:
        database = self.home / "latoile.db"
        with sqlite3.connect(database) as connection:
            rows = connection.execute(
                "SELECT seq, kind, payload FROM event WHERE project_id = ? ORDER BY seq",
                (self.data["project_id"],),
            ).fetchall()
        finished_seq = None
        requested_seq = None
        for seq, kind, raw_payload in rows:
            with contextlib.suppress(json.JSONDecodeError):
                payload = json.loads(raw_payload)
                if payload.get("run_id") != reviewer_run_id:
                    continue
                if kind == "run_finished":
                    finished_seq = int(seq)
                elif kind == "approval_requested":
                    requested_seq = int(seq)
        if finished_seq is None or requested_seq is None or finished_seq >= requested_seq:
            raise CanaryFailure("human review was not requested after the Reviewer finished")
        self.data[f"{prefix}_reviewer_finished_event"] = finished_seq
        self.data[f"{prefix}_approval_requested_event"] = requested_seq

    def wait_for_review(
        self,
        api: Api,
        project_id: str,
        reviewed_run_id: str,
        expected_approvable: bool,
        prefix: str,
    ) -> tuple[dict[str, Any], dict[str, Any]]:
        self.stage(f"{prefix} Reviewer result")
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
                gate = payload.get("gate") or {}
                references = (payload.get("visual_evidence") or {}).get("references") or []
                comparisons = api.get(
                    f"/api/runs/{urllib.parse.quote(reviewed_run_id, safe='')}/visual-comparisons"
                )
                expected_status = "passed" if expected_approvable else "blocking"
                evidence = one_visual_comparison(
                    comparisons, reviewed_run_id, expected_status
                )
                if (
                    payload.get("schema_version") != 2
                    or payload.get("reviewed_run_id") != reviewed_run_id
                    or gate.get("trusted_v2") is not True
                    or gate.get("approvable") is not expected_approvable
                    or len(references) != 1
                    or references[0].get("evidence_id") != evidence.get("id")
                    or references[0].get("baseline_png_digest")
                    != evidence.get("baseline_png_digest")
                ):
                    raise CanaryFailure("Reviewer V2 gate is not bound to the exact evidence")
                if expected_approvable:
                    if verdict not in {"approve", "approve_with_reservations"}:
                        raise CanaryFailure(
                            f"corrected Reviewer returned non-deliverable verdict {verdict!r}"
                        )
                elif verdict != "changes_requested":
                    raise CanaryFailure("blocking evidence did not canonicalize the verdict")
                reviewer_run = api.get(f"/api/runs/{review['run_id']}")
                if reviewer_run.get("status") != "finished":
                    raise CanaryFailure("Reviewer approval appeared before a finished reviewer run")
                self.prove_reviewer_before_human(review["run_id"], prefix)
                self.data[f"{prefix}_reviewer_run_id"] = review["run_id"]
                self.data[f"{prefix}_review_verdict"] = verdict
                self.data[f"{prefix}_review_gate_code"] = gate.get("code")
                self.data[f"{prefix}_visual_evidence_id"] = evidence.get("id")
                self.data[f"{prefix}_visual_status"] = evidence.get("status")
                return review, evidence
            tasks = api.get(f"/api/projects/{project_id}/tasks")
            if tasks:
                latest = tasks[0].get("latest_run_id")
                if latest and latest != reviewed_run_id:
                    reviewer_run = api.get(f"/api/runs/{latest}")
                    if reviewer_run.get("status") in {"error", "cancelled"}:
                        raise CanaryFailure(
                            f"Reviewer ended with status {reviewer_run.get('status')}"
                        )
            time.sleep(1)
        raise CanaryFailure(f"{prefix} Reviewer result exceeded the canary timeout")

    def reject_for_correction(
        self, api: Api, review: dict[str, Any], original_evidence: dict[str, Any]
    ) -> str:
        self.stage("owner rejects the measured visual regression")
        comment = (
            "Remove only the `<style id=\"canary-regression\">` injection from server.mjs. "
            "Serve the exact approved mockup bytes unchanged for every route, keep design/ "
            "immutable, verify the title 'Contrat visuel LaToile', commit the correction and "
            "finish with a clean worktree."
        )
        decided = api.post(
            f"/api/approvals/{review['id']}",
            {"granted": False, "comment": comment},
        )
        corrective_run_id = decided.get("corrective_run_id")
        if (
            decided.get("status") != "rejected"
            or decided.get("decision_comment") != comment
            or not corrective_run_id
        ):
            raise CanaryFailure("visual rejection did not start one audited corrective run")
        detail = api.get(f"/api/approvals/{review['id']}")
        payload = json.loads(detail.get("payload") or "{}")
        references = (payload.get("visual_evidence") or {}).get("references") or []
        if (
            detail.get("corrective_run_id") != corrective_run_id
            or len(references) != 1
            or references[0].get("evidence_id") != original_evidence.get("id")
        ):
            raise CanaryFailure("the original visual decision lost its immutable evidence")
        self.data["rejected_review_id"] = review["id"]
        self.data["corrective_run_id"] = corrective_run_id
        self.data["owner_rejection_recorded"] = True
        return str(corrective_run_id)

    def approve_corrected_review(self, api: Api, review: dict[str, Any]) -> None:
        self.stage("owner approves corrected trusted evidence")
        decided = api.post(
            f"/api/approvals/{review['id']}",
            {
                "granted": True,
                "comment": "Greenfield visual canary correction explicitly approved.",
            },
        )
        if decided.get("status") != "granted":
            raise CanaryFailure("corrected human review approval was not persisted")
        self.data["granted_review_id"] = review["id"]

    def prove_spec_and_baseline_unchanged(
        self,
        api: Api,
        project: dict[str, Any],
        spec: dict[str, Any],
        original: dict[str, Any],
        corrected: dict[str, Any],
    ) -> None:
        self.stage("immutable baseline and correction audit")
        prove_replacement_evidence(original, corrected)
        specs = api.get(f"/api/projects/{project['id']}/spec-versions")
        current = next((item for item in specs if item.get("id") == spec.get("id")), None)
        if (
            not current
            or current.get("status") != "approved"
            or current.get("package_commit_sha") != self.data.get("spec_commit_sha")
            or current.get("manifest_digest") != self.data.get("manifest_digest")
        ):
            raise CanaryFailure("the approved architecture changed during correction")
        baselines = api.get(f"/api/spec-versions/{spec['id']}/baselines")
        if (
            len(baselines) != 1
            or baselines[0].get("png_digest") != original.get("baseline_png_digest")
            or corrected.get("baseline_png_digest") != original.get("baseline_png_digest")
        ):
            raise CanaryFailure("the corrective run did not reuse the immutable baseline")
        checkout = pathlib.Path(project["local_path"])
        checked(
            [
                "git",
                "diff",
                "--quiet",
                str(self.data["spec_commit_sha"]),
                "HEAD",
                "--",
                str(spec["design_dir"]),
            ],
            cwd=checkout,
            label="approved design immutability check",
        )
        self.data["corrected_visual_evidence_id"] = corrected.get("id")
        self.data["baseline_reused_after_correction"] = True

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
        if "Contrat visuel LaToile" not in rendered or "canary-regression" in rendered:
            raise CanaryFailure("the corrected preview does not serve the exact approved mockup")
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
        spec, _validation = self.manager_and_spec(api, project)
        _, executor_run = self.dispatch_executor(api, project)
        self.wait_for_executor(api, executor_run, "initial")
        initial_review, original_evidence = self.wait_for_review(
            api, project["id"], executor_run, False, "initial"
        )
        corrective_run = self.reject_for_correction(
            api, initial_review, original_evidence
        )
        self.wait_for_executor(api, corrective_run, "corrected")
        corrected_review, corrected_evidence = self.wait_for_review(
            api, project["id"], corrective_run, True, "corrected"
        )
        self.prove_spec_and_baseline_unchanged(
            api, project, spec, original_evidence, corrected_evidence
        )
        self.prove_preview(api, project["id"])
        self.approve_corrected_review(api, corrected_review)
        self.deliver(api, project, repo)
        self.data["permission_decisions"] = self.permission_decisions


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
        or evidence.get("schema_version") not in {1, 2}
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
    run = commands.add_parser(
        "run", help="run the opt-in greenfield visual-contract canary"
    )
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
