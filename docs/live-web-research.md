# OpenMindAI — Live Web Search and Advanced Deep Research

Updated: 2026-10-17

## Features and limits

Web Search and Deep Research remain **local-model-first**. Only retrieval uses
the internet. Search uses configured official APIs when available and public
web search pages otherwise; no paid external AI model is required.

- Web Search: fallback between Brave Search API (optional), legacy Google Custom
  Search JSON API (optional), DuckDuckGo HTML and Bing public search HTML.
- Deep Research: four distinct search questions, up to 12 unique sources,
  and up to eight HTML/text pages. Ordinary Web Search fetches three pages.
- Each full page receives a maximum of 1.5 MB of transfer and 2,200
  characters of cleaned text in the prompt. This is an *extract*, not a
  complete unlimited article transcript. Search responses are capped at 1 MB.
- The engine marks a result as snippet-only when its page cannot be fetched.
  Sources are numbered consistently; the model must cite their real links,
  assess independent corroboration and disclose uncertainty.
- Max elapsed retrieval: 35 seconds (search) and 85 seconds (research),
  per-request deadline 9 seconds, concurrent page fetches at most three.
  The user can cancel search while it is running.
- Backend privacy preferences are checked before network retrieval. Web
  Search and Deep Research settings are independent. The Capabilities settings
  panel checks connectivity automatically when opened and every 60 seconds,
  with a manual Check connection button. Disable both modes to turn off
  this probe.

## Provider credentials and availability

The following **optional** environment variables are read by the Rust app
process, never committed to GitHub, inserted into prompts, or exposed to
frontend JavaScript:

- `OPENMINDAI_BRAVE_SEARCH_KEY` — Brave Search API token.
- `OPENMINDAI_GOOGLE_CSE_KEY` — existing Google Custom Search JSON API key.
- `OPENMINDAI_GOOGLE_CSE_ID` — associated Programmable Search engine ID.

Brave Search API may require an account/paid plan; never imply it is a free
anonymous provider. Without credentials the app falls back to DuckDuckGo HTML
and Bing HTML search; both are best-effort and may rate-limit, challenge bots,
change page markup or be unavailable in a country. The old Microsoft Bing
Search **API** was retired August 11, 2025; the implementation does not invoke
it. Google Custom Search JSON API is closed to new users and scheduled to
discontinue January 1, 2027. Do not rely on Google as a future default.

Provider and official product details:
- https://api-dashboard.search.brave.com/documentation/guides/authentication
- https://developers.google.com/custom-search/v1/overview
- https://learn.microsoft.com/lifecycle/announcements/bing-search-api-retirement

## Security

A retrieved page is untrusted data, not an instruction to the assistant.
No JavaScript is run. Fetches are limited to HTML, XHTML and plain text.

Before fetching, the application rejects direct private/loopback/link-local
URLs, resolves every hostname and rejects **any** nonpublic returned address.
The approved public IPs are pinned for the request. Redirects are disabled to
avoid crossing into local/private networks via a malicious redirect. Page
response types, sizes, timeouts and cancellation are bounded. Some proxies
resolve hostnames independently, so system proxies should be trusted.

Search query text is sent to external providers only when the corresponding
feature is enabled. API keys should be provided by the deployment environment
or future secret-store UI, not placed in a source-controlled configuration.

## Verification and future enhancements

- Automated unit tests cover JSON results, Bing HTML fixture parsing,
  URL deduplication, text extraction, and private-IP rejection.
- CI validates TypeScript + frontend + Rust on Windows, Linux and macOS.
- Tests do **not** prove live provider success on every user's network:
  complete a live Windows smoke test including offline, proxy, Brave/Google
  credentials, search engine rate limits, and malformed HTML.
- PDF files and JS-only websites are not full-page extracted. Full factual
  cross-verification cannot be guaranteed by an offline 4B model; where claims
  conflict, the assistant must describe both with citations.
- Source snippets might be inaccurate even when returned by a search engine.
  Answer quality should not be overstated as equivalent to ChatGPT's cloud web
  search or the work of professional research fact-checkers.
