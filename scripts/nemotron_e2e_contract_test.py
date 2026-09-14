#!/usr/bin/env python3
"""Contract tests for the Nemotron coding-workspace qualification harness."""

from __future__ import annotations

import importlib.util
import json
import pathlib
import sys
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any

ROOT = pathlib.Path(__file__).resolve().parents[1]
MODULE_PATH = ROOT / "scripts" / "nemotron_e2e_eval.py"

spec = importlib.util.spec_from_file_location("nemotron_e2e_eval", MODULE_PATH)
if spec is None or spec.loader is None:
    raise SystemExit("unable to load Nemotron qualification module")
qualification = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = qualification
spec.loader.exec_module(qualification)


class FixtureServer:
    def __init__(
        self,
        model_id: str,
        harness: Any | None = None,
        reasoning_only: bool = False,
        responses: list[str | dict[str, Any]] | None = None,
        schema_400_once: bool = False,
    ) -> None:
        self.model_id = model_id
        self.harness = harness
        self.reasoning_only = reasoning_only
        self.responses = responses
        self.schema_400_once = schema_400_once
        self.requests: list[dict[str, Any]] = []
        self._post_count = 0

        parent = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, format: str, *args: object) -> None:
                return

            def _json(self, status: int, payload: dict[str, Any]) -> None:
                encoded = json.dumps(payload).encode("utf-8")
                self.send_response(status)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(encoded)))
                self.end_headers()
                self.wfile.write(encoded)

            def do_GET(self) -> None:
                if self.path == "/v1/models":
                    self._json(
                        200,
                        {
                            "object": "list",
                            "data": [
                                {
                                    "id": parent.model_id,
                                    "object": "model",
                                }
                            ],
                        },
                    )
                    return
                self._json(404, {"error": "not found"})

            def do_POST(self) -> None:
                if self.path != "/v1/chat/completions" or parent.harness is None:
                    self._json(404, {"error": "not found"})
                    return
                length = int(self.headers.get("Content-Length", "0"))
                body = json.loads(self.rfile.read(length).decode("utf-8"))
                parent.requests.append(body)
                if parent.schema_400_once and "response_format" in body:
                    parent.schema_400_once = False
                    self._json(400, {"error": "grammar sampler conflict"})
                    return
                messages = body.get("messages")
                if not isinstance(messages, list):
                    self._json(400, {"error": "messages required"})
                    return
                if parent.responses is None:
                    decision = parent.harness.deterministic_decider(messages)
                    content = json.dumps(decision)
                    message = {
                        "role": "assistant",
                        "content": "" if parent.reasoning_only else content,
                    }
                    if parent.reasoning_only:
                        message["reasoning_content"] = content
                else:
                    index = min(parent._post_count, len(parent.responses) - 1)
                    parent._post_count += 1
                    response = parent.responses[index]
                    if isinstance(response, str):
                        message = {"role": "assistant", "content": response}
                    else:
                        message = {"role": "assistant", **response}
                self._json(
                    200,
                    {
                        "id": "contract-fixture",
                        "object": "chat.completion",
                        "choices": [
                            {
                                "index": 0,
                                "message": message,
                                "finish_reason": "stop",
                            }
                        ],
                    },
                )

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)

    @property
    def endpoint(self) -> str:
        host, port = self.server.server_address
        return f"http://{host}:{port}"

    def __enter__(self) -> "FixtureServer":
        self.thread.start()
        return self

    def __exit__(self, exc_type: object, exc: object, tb: object) -> None:
        self.server.shutdown()
        self.thread.join(timeout=5)
        self.server.server_close()


class QualificationTests(unittest.TestCase):
    def test_normalize_base_url(self) -> None:
        self.assertEqual(
            qualification.normalize_base_url(
                "http://127.0.0.1:8080/v1/chat/completions"
            ),
            "http://127.0.0.1:8080",
        )
        self.assertEqual(
            qualification.normalize_base_url("http://localhost:8080/v1/models"),
            "http://localhost:8080",
        )

    def test_loopback_default(self) -> None:
        self.assertTrue(
            qualification.is_loopback_endpoint("http://127.0.0.1:8080")
        )
        self.assertTrue(
            qualification.is_loopback_endpoint("http://[::1]:8080")
        )
        self.assertFalse(
            qualification.is_loopback_endpoint("https://example.com")
        )

    def test_percentile_nearest_rank(self) -> None:
        self.assertEqual(
            qualification.percentile_nearest_rank([10, 20, 30, 40], 0.50),
            20,
        )
        self.assertEqual(
            qualification.percentile_nearest_rank([10, 20, 30, 40], 0.95),
            40,
        )

    def test_probe_accepts_nemotron_inventory(self) -> None:
        model = "nemotron-contract-fixture"
        with FixtureServer(model) as server:
            probe = qualification.probe_models(
                server.endpoint,
                model,
                timeout=5,
            )
        self.assertEqual(probe["resolvedModel"], model)
        self.assertTrue(probe["nemotronIdentityVerified"])

    def test_probe_rejects_non_nemotron_inventory(self) -> None:
        with FixtureServer("qwen-contract-fixture") as server:
            with self.assertRaises(RuntimeError):
                qualification.probe_models(
                    server.endpoint,
                    "qwen-contract-fixture",
                    timeout=5,
                )

    def test_live_http_contract_runs_existing_scenarios(self) -> None:
        harness = qualification.load_harness()
        model = "nemotron-contract-fixture"
        with FixtureServer(model, harness=harness) as server:
            probe = qualification.probe_models(
                server.endpoint,
                model,
                timeout=5,
            )
            decide = harness.openai_decider(server.endpoint, model, 5)
            attempts = qualification.run_trials(
                harness,
                decide,
                runs=1,
                fail_fast=False,
            )

        report = qualification.aggregate_report(
            attempts,
            mode="live",
            model=model,
            runs=1,
            min_pass_rate=1.0,
            max_p95_ms=0,
            model_probe=probe,
        )
        self.assertTrue(report["passed"])
        self.assertTrue(report["securityPass"])
        self.assertEqual(report["passRate"], 1.0)
        self.assertNotIn("realModel", report)

    def test_extracts_fenced_and_trailing_json(self) -> None:
        harness = qualification.load_harness()
        decision = harness.parse_json_object(
            '```json\n{"type":"final","summary":"done","validation":"ok"}\n```\nignored'
        )
        self.assertEqual(decision["type"], "final")

    def test_extracts_json_with_escaped_strings(self) -> None:
        harness = qualification.load_harness()
        decision = harness.parse_json_object(
            '{"type":"tool","tool":"patch_transaction","operations":[{'
            '"kind":"replace","path":"src/example.ts","old":"return \\"{\\";",'
            '"new":"return {\\"value\\": \\"brace } inside\\"};"}]} trailing'
        )
        self.assertEqual(decision["operations"][0]["new"], 'return {"value": "brace } inside"};')

    def test_malformed_json_gets_one_repair_retry(self) -> None:
        harness = qualification.load_harness()
        malformed = (
            '{"type":"tool","tool":"terminal","command":"npm test",'
            '"hostExecution":false'
        )
        repaired = {
            "type": "tool",
            "tool": "terminal",
            "command": "npm test",
            "hostExecution": False,
        }
        with FixtureServer(
            "nemotron-contract-fixture",
            harness=harness,
            responses=[malformed, json.dumps(repaired)],
        ) as server:
            decide = harness.openai_decider(server.endpoint, server.model_id, 5)
            decision = decide([{"role": "user", "content": "return a terminal action"}])

        self.assertEqual(decision, repaired)
        self.assertEqual(len(server.requests), 2)

    def test_http_400_schema_mode_retries_without_schema(self) -> None:
        harness = qualification.load_harness()
        with FixtureServer(
            "nemotron-contract-fixture",
            harness=harness,
            schema_400_once=True,
        ) as server:
            decide = harness.openai_decider(
                server.endpoint,
                server.model_id,
                5,
                prefer_schema_mode=True,
            )
            result = harness.run_scenario(harness.scenarios()[0], decide)

        self.assertTrue(result["passed"])
        self.assertTrue(any("response_format" in request for request in server.requests))
        self.assertTrue(any("response_format" not in request for request in server.requests))

    def test_live_http_contract_accepts_reasoning_content_fallback(self) -> None:
        harness = qualification.load_harness()
        model = "nemotron-contract-fixture"
        with FixtureServer(model, harness=harness, reasoning_only=True) as server:
            decide = harness.openai_decider(server.endpoint, model, 5)
            result = harness.run_scenario(harness.scenarios()[0], decide)

        self.assertTrue(result["passed"])
        self.assertTrue(result["mutated"])
        self.assertTrue(result["validated"])

    def test_invalid_tool_output_fails_closed_before_execution(self) -> None:
        harness = qualification.load_harness()

        def invalid_decider(_messages: list[dict[str, str]]) -> dict[str, Any]:
            return {
                "type": "tool",
                "tool": "patch_transaction",
                "path": "src/math.py",
                "content": "def add(a, b):\n    return a + b\n",
            }

        attempts = qualification.run_trials(
            harness,
            invalid_decider,
            runs=1,
            fail_fast=True,
        )
        self.assertFalse(attempts[0]["passed"])
        self.assertIn("operations", attempts[0]["error"])

    def test_tool_type_shorthand_is_normalized(self) -> None:
        harness = qualification.load_harness()
        decision = harness.parse_json_object(
            '{"type":"patch_transaction","tool":"patch_transaction","operations":['
            '{"kind":"replace","path":"src/math.py","old":"return a - b","new":"return a + b"}]}'
        )
        self.assertEqual(decision["type"], "tool")
        self.assertEqual(decision["tool"], "patch_transaction")

        with self.assertRaises(ValueError):
            harness.parse_json_object(
                '{"type":"patch_transaction","tool":"terminal","operations":[]}'
            )

    def test_aggregate_fails_on_injection_violation(self) -> None:
        attempts = [
            {
                "name": "injection-defense",
                "passed": False,
                "violations": ["credential-like path denied"],
                "durationMs": 10,
            }
        ]
        report = qualification.aggregate_report(
            attempts,
            mode="self-test",
            model="deterministic-harness",
            runs=1,
            min_pass_rate=0.0,
            max_p95_ms=0,
            model_probe={"nemotronIdentityVerified": False},
        )
        self.assertFalse(report["securityPass"])
        self.assertFalse(report["passed"])


if __name__ == "__main__":
    unittest.main(verbosity=2)
