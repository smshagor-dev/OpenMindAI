#!/usr/bin/env python3
"""Offline, reproducible evaluator for Web Search and Deep Research runs.

Input: JSON with cases containing expected_relevant_urls, results, claims, and
optional provider_attempts, latency_ms, cancelled. No network or dependencies.
"""
import argparse
import json
import statistics
from pathlib import Path
from urllib.parse import urlsplit, urlunsplit


def canonical_url(value):
    try:
        p = urlsplit(value)
        if p.scheme not in ("https", "http") or not p.hostname:
            return None
        return urlunsplit((p.scheme.lower(), p.netloc.lower(), p.path.rstrip("/") or "/", p.query, ""))
    except (TypeError, ValueError):
        return None


def evaluate(data):
    cases = data.get("cases", [])
    if not isinstance(cases, list) or not cases:
        raise ValueError("cases must be a nonempty array")
    totals = dict(claims=0, cited_claims=0, supported_cited_claims=0,
                  unsupported_claims=0, provider_attempts=0, provider_successes=0,
                  primary_failures=0, fallback_recoveries=0, relevant_hits=0,
                  expected_relevant=0, retrieved=0, duplicate_results=0,
                  cancelled=0, cancellation_successes=0)
    latencies = []
    for case in cases:
        results = case.get("results", [])
        if not isinstance(results, list):
            raise ValueError("results must be an array")
        urls = [canonical_url(r.get("url")) for r in results]
        urls = [u for u in urls if u]
        expected = {u for x in case.get("expected_relevant_urls", []) if (u := canonical_url(x))}
        totals["retrieved"] += len(urls)
        totals["duplicate_results"] += len(urls) - len(set(urls))
        totals["expected_relevant"] += len(expected)
        totals["relevant_hits"] += len(set(urls) & expected)
        for claim in case.get("claims", []):
            label = claim.get("label")
            if label not in ("supported", "contradicted", "unsupported", "unverifiable"):
                raise ValueError("claim label must be supported, contradicted, unsupported or unverifiable")
            totals["claims"] += 1
            if label != "supported":
                totals["unsupported_claims"] += 1
            if claim.get("cited", False):
                totals["cited_claims"] += 1
                if label == "supported":
                    totals["supported_cited_claims"] += 1
        attempts = case.get("provider_attempts", [])
        totals["provider_attempts"] += len(attempts)
        totals["provider_successes"] += sum(bool(a.get("usable_results")) for a in attempts)
        if attempts and not attempts[0].get("usable_results"):
            totals["primary_failures"] += 1
            if any(a.get("usable_results") for a in attempts[1:]):
                totals["fallback_recoveries"] += 1
        if case.get("cancelled"):
            totals["cancelled"] += 1
            totals["cancellation_successes"] += bool(case.get("stopped_without_completed_footer"))
        if case.get("latency_ms") is not None:
            latency = case["latency_ms"]
            if not isinstance(latency, (int, float)) or latency < 0:
                raise ValueError("latency_ms must be nonnegative")
            latencies.append(latency)

    def ratio(a, b):
        return round(a / b, 4) if b else None

    ordered = sorted(latencies)
    p95 = ordered[max(0, __import__("math").ceil(0.95 * len(ordered)) - 1)] if ordered else None
    return {"case_count": len(cases), "counts": totals, "metrics": {
        "citation_precision": ratio(totals["supported_cited_claims"], totals["cited_claims"]),
        "unsupported_claim_rate": ratio(totals["unsupported_claims"], totals["claims"]),
        "provider_success_rate": ratio(totals["provider_successes"], totals["provider_attempts"]),
        "fallback_recovery_rate": ratio(totals["fallback_recoveries"], totals["primary_failures"]),
        "relevant_url_recall": ratio(totals["relevant_hits"], totals["expected_relevant"]),
        "duplicate_fraction": ratio(totals["duplicate_results"], totals["retrieved"]),
        "cancellation_success_rate": ratio(totals["cancellation_successes"], totals["cancelled"]),
        "latency_median_ms": statistics.median(latencies) if latencies else None,
        "latency_p95_ms": p95,
    }}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("input", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    report = evaluate(json.loads(args.input.read_text(encoding="utf-8")))
    output = json.dumps(report, indent=2, sort_keys=True) + "\n"
    if args.output:
        args.output.write_text(output, encoding="utf-8")
    else:
        print(output, end="")


if __name__ == "__main__":
    main()
