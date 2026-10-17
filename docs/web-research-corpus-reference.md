# Advanced Web Search and Deep Research — implementation reference

This document points code-reuse tools and reviewers to the original, already-merged implementation rather than duplicating it in a new pull request.

## Original implementation

- [PR #259 — Advanced web research: provider fallback, full-page evidence and network health](https://github.com/smshagor-dev/OpenMindAI/pull/259)
- [PR #260 — Safe, host-generated citations and Bing RSS fallback](https://github.com/smshagor-dev/OpenMindAI/pull/260)
- [PR #261 — Provider reliability and secure search configuration](https://github.com/smshagor-dev/OpenMindAI/pull/261)

## Design overview

The implementation combines optional configured search providers with keyless fallback, bounded multi-page evidence retrieval, URL and DNS validation against private-network targets, cancellation and timeouts, and source links derived from actual retrieval results. Research results must be treated as untrusted external input, not as instructions.

## Reuse and attribution

Source code is in this repository under its Apache-2.0 license. Preserve applicable license and attribution notices. For exact source changes and authorship history, use the linked pull requests and commits, not this summary.

This is a reference document only; it does not reintroduce or modify the merged search implementation, and it does not claim independent live-provider verification.
