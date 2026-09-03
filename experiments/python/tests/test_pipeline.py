from dataclasses import replace
import json
import math
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from openmind_personalization import LearningExample, split_examples
from openmind_personalization.activation import activate, active_path, rollback
from openmind_personalization.pipeline import atomic_json, load_feedback, sha256, TrainingOptions


class PipelineTests(unittest.TestCase):
    def test_duplicate_feedback_cannot_cross_split(self):
        one = LearningExample("same question", "old", "new", "p", "2026-09-01")
        two = replace(one, user_input=" SAME   question ", preferred_output="corrected", created_at="2026-09-03")
        train, holdout = split_examples([one, two])
        self.assertEqual(len(train) + len(holdout), 1)
        self.assertEqual((train + holdout)[0].preferred_output, "corrected")

    def test_approval_and_profile_are_required(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "feedback.jsonl"
            path.write_text(json.dumps({"approved": False}))
            with self.assertRaises(ValueError):
                load_feedback(path, "p")
            path.write_text(json.dumps({"approved": True, "profile_id": "other", "user_input": "q",
                                       "assistant_output": "a", "preferred_output": "b", "created_at": "today"}))
            with self.assertRaises(ValueError):
                load_feedback(path, "p")

    def test_activation_rejection_is_atomic_and_rollback_restores_base(self):
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            base, adapter = directory / "base.gguf", directory / "adapter.gguf"
            base.write_bytes(b"GGUFbase"); adapter.write_bytes(b"GGUFadapter")
            candidate = directory / "candidate.json"
            value = {"schema_version": 1, "profile_id": "p", "model_id": "nano",
                     "base_sha256": sha256(base), "adapter_path": str(adapter), "adapter_sha256": sha256(adapter),
                     "probe_prompt": "hi", "evaluation": {"accepted": True, "baseline_loss": 3.0,
                     "candidate_loss": 2.0, "holdout_examples": 12}}
            atomic_json(candidate, value)
            active_dir = directory / "active"
            args = dict(directory=active_dir, profile_id="p", model_id="nano", base_gguf=base, worker=base)
            with patch("openmind_personalization.activation.probe_native", side_effect=ValueError("failed")):
                with self.assertRaises(ValueError):
                    activate(candidate, **args)
                self.assertFalse(active_path(active_dir, "nano").exists())
            with patch("openmind_personalization.activation.probe_native"):
                target = activate(candidate, **args)
                self.assertEqual(json.loads(target.read_text())["candidate"], str(candidate))
                rollback(**args)
                self.assertIsNone(json.loads(target.read_text())["candidate"])
                value["evaluation"]["candidate_loss"] = 4
                atomic_json(candidate, value)
                with self.assertRaises(ValueError):
                    activate(candidate, **args)
                self.assertIsNone(json.loads(target.read_text())["candidate"])

    def test_nonfinite_training_config_rejected(self):
        with self.assertRaises(ValueError):
            TrainingOptions(learning_rate=math.nan).validate()
        with self.assertRaises(ValueError):
            TrainingOptions(max_rss_gb=math.inf).validate()
