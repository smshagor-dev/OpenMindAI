# Web Search & Deep Research — production quality gates

This document defines acceptance criteria; it is **not** evidence that the gates have passed.

## Live end-to-end verification

Run on supported Windows/Linux/macOS targets where available, with an explicitly permitted external network.

| Scenario | Required outcome |
|---|---|
| Keyless DuckDuckGo/Bing search | Valid source URLs or an explicit retrieval failure, never fabricated citations |
| Brave valid and invalid credentials | Correct results; invalid credentials fall back without leaking keys |
| HTTP 429 and HTTP 5xx | Bounded cooldown/retry; healthy provider fallback |
| Offline mode | Local chat works; no claims of verified live evidence |
| Cancellation during search and page fetch | Prompt termination; no completed-answer source footer |
| Search and research disabled | No outbound retrieval or health probes |
| Private/loopback/metadata and DNS rebinding | Requests blocked before any sensitive network access |
| Proxy-only network | No SSRF bypass; explain when full-page enrichment is unavailable |
| Multilingual queries | Bangla, English and Russian results evaluated separately |

Record OS, app commit, timestamp, provider, credential-presence (never secrets), status, latency, result count and sanitized failure reason.

## Citation quality

For each answer, label atomic factual claims as supported, contradicted, unsupported or unverifiable against the exact fetched passage. Host-generated URLs alone are not claim-level verification. Fail evaluation if a fabricated URL is shown or an unsupported claim is described as verified.

## Retrieval and evidence quality

Evaluate relevance ranking, duplicate URL and near-duplicate text detection, cross-domain corroboration, contradiction handling, freshness and source diversity. Preserve provenance for snippet-only evidence; display progress and provider errors without exposing secrets.

For larger research, enforce configurable limits for query count, page count, concurrency, total bytes, elapsed time and cancellation. Never make page-count expansion unbounded.

## Benchmark methodology

Use a versioned dataset of factual queries, multi-hop research tasks, conflicting-source tasks, stale-information traps and adversarial HTML. Include English, Bangla and Russian.

Metrics:
- Provider success rate = searches with usable results / attempted searches.
- Fallback recovery rate = primary failures recovered by another provider / primary failures.
- Citation precision = supported cited claims / all cited claims.
- Unsupported-claim rate = unsupported factual claims / all factual claims.
- Source relevance at k, unique-domain coverage and duplicate fraction.
- Median, p95 latency and timeout/cancellation success.
- Memory, transferred bytes and per-task retrieved-token budget.

Keep fixtures, evaluator decisions, sample sizes and raw sanitized measurements with each report. Never infer live-provider success from unit tests.

## Release gate

Require green CI, repeatable local tests, live-network smoke results, security checks, no leaked credentials and a reviewed citation-quality report before describing research as production-verified.
