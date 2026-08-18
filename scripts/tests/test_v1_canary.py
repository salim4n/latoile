import importlib.util
import json
import pathlib
import sqlite3
import tempfile
import unittest
from unittest import mock


SCRIPT = pathlib.Path(__file__).parents[1] / "v1_canary.py"
SPEC = importlib.util.spec_from_file_location("v1_canary", SCRIPT)
canary = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(canary)


class CanarySafetyTests(unittest.TestCase):
    def test_redaction_removes_known_secrets_and_hidden_reasoning(self):
        output = canary.redact(
            "token=secret-value\nreasoning_content=private chain\nresult=ok",
            ["secret-value"],
        )
        self.assertNotIn("secret-value", output)
        self.assertNotIn("private chain", output)
        self.assertIn("[redacted]", output)
        self.assertIn("result=ok", output)

    def test_cleanup_refuses_a_non_canary_repository(self):
        with tempfile.TemporaryDirectory() as directory:
            evidence = pathlib.Path(directory) / "evidence.json"
            evidence.write_text(
                json.dumps({"schema_version": 1, "repository": "owner/production"})
            )
            args = mock.Mock(evidence=str(evidence), confirm="owner/production", delete=False)
            with mock.patch.object(canary, "checked") as checked:
                self.assertEqual(canary.cleanup(args), 2)
                checked.assert_not_called()

    def test_cleanup_archives_after_exact_confirmation_and_preserves_evidence(self):
        with tempfile.TemporaryDirectory() as directory:
            evidence = pathlib.Path(directory) / "evidence.json"
            repo = "owner/latoile-v1-canary-20260818"
            evidence.write_text(json.dumps({"schema_version": 1, "repository": repo}))
            args = mock.Mock(evidence=str(evidence), confirm=repo, delete=False)
            with mock.patch.object(canary, "checked", return_value="") as checked:
                self.assertEqual(canary.cleanup(args), 0)
                checked.assert_called_once_with(
                    ["gh", "repo", "archive", repo, "--yes"],
                    label="GitHub repository archive",
                )
            saved = json.loads(evidence.read_text())
            self.assertTrue(saved["cleanup"]["remote_retained"])
            self.assertTrue(saved["cleanup"]["remote_archived"])
            self.assertIn("remote_archived_at", saved["cleanup"])

    def test_cleanup_accepts_the_greenfield_evidence_schema(self):
        with tempfile.TemporaryDirectory() as directory:
            evidence = pathlib.Path(directory) / "evidence.json"
            repo = "owner/latoile-v1-canary-20260818-greenfield"
            evidence.write_text(json.dumps({"schema_version": 2, "repository": repo}))
            args = mock.Mock(evidence=str(evidence), confirm=repo, delete=False)
            with mock.patch.object(canary, "checked", return_value=""):
                self.assertEqual(canary.cleanup(args), 0)

    def test_visual_transition_requires_blocking_then_distinct_passed_evidence(self):
        original = {
            "id": "visual:run-1:home",
            "run_id": "run-1",
            "comparison_id": "home",
            "status": "blocking",
            "manifest_digest": "a" * 64,
            "baseline_png_digest": "b" * 64,
            "render_png_digest": "c" * 64,
            "pixel_diff_digest": "d" * 64,
            "heatmap_png_digest": "e" * 64,
        }
        corrected = {
            **original,
            "id": "visual:run-2:home",
            "run_id": "run-2",
            "status": "passed",
            "render_png_digest": "1" * 64,
            "pixel_diff_digest": "2" * 64,
            "heatmap_png_digest": "3" * 64,
        }
        self.assertEqual(
            canary.one_visual_comparison([original], "run-1", "blocking"),
            original,
        )
        self.assertEqual(
            canary.one_visual_comparison([corrected], "run-2", "passed"),
            corrected,
        )
        canary.prove_replacement_evidence(original, corrected)

        with self.assertRaises(canary.CanaryFailure):
            canary.prove_replacement_evidence(original, {**corrected, "id": original["id"]})
        with self.assertRaises(canary.CanaryFailure):
            canary.prove_replacement_evidence(
                original, {**corrected, "baseline_png_digest": "f" * 64}
            )

    def test_visual_observation_keeps_metrics_without_unbounded_payloads(self):
        evidence = {
            "id": "visual:run-2:home",
            "run_id": "run-2",
            "status": "reservation",
            "changed_pixels": 0,
            "total_pixels": 329_160,
            "pixel_ratio_micros": 0,
            "max_geometry_delta_milli": 0,
            "accessibility_changes": 2,
            "failure_message": "agent-controlled and intentionally omitted",
        }

        observation = canary.visual_evidence_observation(evidence)

        self.assertEqual(observation["status"], "reservation")
        self.assertEqual(observation["accessibility_changes"], 2)
        self.assertNotIn("failure_message", observation)

    def test_reviewer_observation_keeps_only_server_binding_fields(self):
        observation = canary.reviewer_observation(
            {
                "schema_version": 2,
                "reviewed_run_id": "run-2",
                "verdict": "approve",
                "summary": "provider prose omitted",
                "findings": [
                    {"severity": "blocking", "text": "omitted"},
                    {"severity": "reservation", "text": "omitted"},
                ],
                "gate": {"trusted_v2": True, "approvable": True, "code": "trusted"},
                "visual_evidence": {
                    "references": [{"evidence_id": "visual:run-2:home", "status": "passed"}]
                },
            }
        )

        self.assertEqual(observation["gate"]["code"], "trusted")
        self.assertEqual(observation["evidence_ids"], ["visual:run-2:home"])
        self.assertEqual(observation["finding_count"], 2)
        self.assertEqual(
            observation["finding_severities"], {"blocking": 1, "reservation": 1}
        )
        self.assertNotIn("summary", observation)
        self.assertNotIn("text", json.dumps(observation))

    def test_baseline_observation_keeps_codes_without_browser_or_provider_text(self):
        observation = canary.baseline_observations(
            [
                {
                    "comparison_id": "home",
                    "status": "failed",
                    "failure_code": "readiness_timeout",
                    "failure_message": "unbounded browser output",
                    "recovery_action": "fix readiness selector",
                    "html": "private mockup",
                }
            ]
        )

        self.assertEqual(observation[0]["failure_code"], "readiness_timeout")
        self.assertNotIn("failure_message", observation[0])
        self.assertNotIn("html", observation[0])

    def test_architecture_diagnostic_is_bounded_and_excludes_owner_content(self):
        with tempfile.TemporaryDirectory() as artifact_root:
            runner = canary.Canary(
                mock.Mock(artifact_root=artifact_root, provider="codex")
            )
            self.addCleanup(runner.close)
            runner.home.mkdir(parents=True)
            runner.known_secrets.append("secret-value")
            with sqlite3.connect(runner.home / "latoile.db") as connection:
                connection.executescript(
                    """
                    CREATE TABLE architecture_session (
                        id TEXT PRIMARY KEY,
                        status TEXT NOT NULL,
                        phase TEXT NOT NULL,
                        package_status TEXT NOT NULL,
                        failure_reason TEXT,
                        skill_name TEXT,
                        operating_mode TEXT,
                        created_at TEXT NOT NULL
                    );
                    CREATE TABLE architecture_question (
                        session_id TEXT NOT NULL,
                        status TEXT NOT NULL,
                        prompt TEXT NOT NULL,
                        answer TEXT
                    );
                    """
                )
                connection.execute(
                    "INSERT INTO architecture_session VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                    (
                        "architecture-1",
                        "failed",
                        "domain_discovery",
                        "not_started",
                        "provider failed with secret-value",
                        "app-architect-brainstorm",
                        "greenfield",
                        "2026-08-18T20:00:00Z",
                    ),
                )
                connection.executemany(
                    "INSERT INTO architecture_question VALUES (?, ?, ?, ?)",
                    (
                        ("architecture-1", "answered", "private prompt", "private answer"),
                        ("architecture-1", "open", "next private prompt", None),
                    ),
                )

            diagnostic = runner.safe_architecture_diagnostic()

            self.assertEqual(diagnostic["session_id"], "architecture-1")
            self.assertEqual(diagnostic["question_counts"], {"answered": 1, "open": 1})
            self.assertIn("[redacted]", diagnostic["failure_reason"])
            serialized = json.dumps(diagnostic)
            self.assertNotIn("private prompt", serialized)
            self.assertNotIn("private answer", serialized)
            self.assertNotIn("secret-value", serialized)


if __name__ == "__main__":
    unittest.main()
