# Validated dataset inventory (partial #327)

The desktop dataset manager no longer relies only on in-memory completion
status. It queries the backend for the visible dataset IDs after a search
changes. The backend checks a bounded set of IDs (up to 200) against the
current portable root: canonical by-id location, bounded manifest, correct
datasetId/source/revision, non-empty file entries, safe paths and exact file
sizes. Missing files, damaged manifests, final symlinks and paths outside
the canonical dataset folder are excluded.

Regression tests cover restart discovery, deleted/truncated dataset files,
wrong-owner manifest and malicious relative file paths. The UI ignores
responses from superseded searches and clears unverified badges on errors.

**Validation limitation:** this is filesystem metadata validation, not a
full SHA-256 rehash of potentially multi-gigabyte datasets. Legacy folders
await successful migration by the normal downloader. Live external file edits
between refreshes are not pushed; explicit on-focus refresh, checksum
verification and portable-volume watchers are still outstanding. No dataset
is deleted by this patch.
