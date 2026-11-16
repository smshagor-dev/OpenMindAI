import importlib.util
import unittest
from pathlib import Path

path = Path(__file__).resolve().parents[1] / "scripts" / "evaluate-web-research.py"
spec = importlib.util.spec_from_file_location("research_eval", path)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class ResearchEvaluationTests(unittest.TestCase):
    def test_retrieval_claims_and_fallback(self):
        report = module.evaluate({"cases": [{
            "results": [{"url": "https://example.org/a"}, {"url": "https://example.org/a/"}],
            "expected_relevant_urls": ["https://example.org/a"],
            "claims": [{"label": "supported", "cited": True},
                       {"label": "unsupported", "cited": True}],
            "provider_attempts": [{"usable_results": False}, {"usable_results": True}],
            "latency_ms": 100, "cancelled": True,
            "stopped_without_completed_footer": True
        }]})
        m = report["metrics"]
        self.assertEqual(m["citation_precision"], 0.5)
        self.assertEqual(m["fallback_recovery_rate"], 1.0)
        self.assertEqual(m["duplicate_fraction"], 0.5)
        self.assertEqual(m["relevant_url_recall"], 1.0)
        self.assertEqual(m["cancellation_success_rate"], 1.0)

    def test_empty_denominators_and_invalid_claims(self):
        m = module.evaluate({"cases": [{}]})["metrics"]
        self.assertIsNone(m["citation_precision"])
        with self.assertRaises(ValueError):
            module.evaluate({"cases": [{"claims": [{"label": "verified"}]}]})

    def test_rejects_non_web_urls(self):
        self.assertIsNone(module.canonical_url("file:///etc/passwd"))
        self.assertIsNone(module.canonical_url("javascript:alert(1)"))


if __name__ == "__main__":
    unittest.main()
