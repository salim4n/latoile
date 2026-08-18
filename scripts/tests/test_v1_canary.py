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


if __name__ == "__main__":
    unittest.main()
