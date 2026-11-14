# GitHub account list pagination (#396)

The repository and issue list commands now fetch pages of 100 from the original
GitHub API endpoint until a short/empty page is returned. Each request retains the
sort/state filters and authentication. Results retain first-seen order and dedupe
by GitHub's stable numeric ID, which prevents duplicate entries when items move
between pages. An entire repeated full page fails explicitly instead of looping.
A later-page HTTP/rate-limit or JSON error fails the call rather than returning a
silently truncated success. Issues continue to include pull-request summaries.
Repository names must be exactly two safe nonempty path components; slashes,
query strings and fragments cannot alter the issue endpoint's filters.

No server-supplied Link URL is followed, so Link cannot redirect the account token
to an unrelated host. Tests deliberately supply a hostile next Link while serving
three local pages and verify request paths/filters and synthetic authorization on
every page. Other fixtures cover a full last page followed by empty, empty first
page, overlapping IDs, repeated pages and later-page HTTP/JSON errors. Five actual
production helper tests, focused Clippy -D warnings, format, locked root metadata
and diff checks passed locally. A three-OS CI harness is included.

Limitations: no live personal GitHub token/account or desktop UI smoke was used.
Full Tauri/native compilation is deferred to CI. GitHub offset pagination is not
an atomic snapshot: concurrent remote changes may still omit/reorder items, even
with deduplication. Large accounts require more API quota, latency and memory;
commands still return a complete Vec rather than a lazy page UI. Rate-limit retry,
progress and cancellation UI are not added. No token/secret-storage/schema or root
lockfile dependency changes are included. Existing RustSec/junction failures are
separate and CI is not claimed all-green.
