# Bounded project catalog aggregation (partial #401)

The catalog was already cursor-paged and excluded large file content, but
used two correlated COUNT subqueries for every selected project row. This
change materializes only the bounded page first, aggregates conversation and
file counts for that selected set, then left-joins the aggregates to avoid
multiplying counts when a project has multiple conversations and files.

Regression tests cover multiple child records, zero-child projects, equal
timestamps across cursor pages, omitted private file content, page-size
clamping and invalid cursors.

Not benchmarked: SQLite query planner behavior on real large datasets,
index-selectivity profiling, desktop UI switching, and query execution
under concurrent writes. Existing project detail loads remain lazy.
