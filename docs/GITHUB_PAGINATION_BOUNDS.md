# GitHub pagination response bounds (partial #396)

The GitHub explorer already paginates repos/issues on main, but its paging
collector previously deserialized each HTTP JSON response without an upper
body-size limit and could iterate indefinitely while every page returned new
IDs. This change caps each decoded page at 8 MiB, including chunked/unknown
Content-Length responses, and stops after 10,000 pages with an explicit error
rather than returning a misleading partial result.

Regression tests include a local HTTP fixture with a page larger than the
limit, verifying a clear error and no additional request, plus guard constants.

Remaining #396 scope: interactive server-side filtering and cursor-based UI,
very large account navigation, and real desktop integration testing.
Cargo tests and cross-platform behavior must be validated before merge.
