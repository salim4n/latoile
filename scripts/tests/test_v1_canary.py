import importlib.util
import json
import pathlib
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


if __name__ == "__main__":
    unittest.main()
