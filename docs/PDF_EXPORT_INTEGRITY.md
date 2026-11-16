# PDF export integrity: partial #347

StagedOutput now validates PDFs before capability-confined publication. After the
magic check it parses the same opened source file with the existing pinned lopdf
0.44 dependency, checks the terminal EOF marker, rejects encrypted output, verifies
the catalog and a nonempty page tree with consistent descendant counts, and checks
referenced content streams can be decoded into PDF operations. Iterative traversal
rejects repeated/cyclic page nodes without recursive tree walking. The source is
rewound for publication, so validation and copy do not reopen different pathnames.
Failures use a generic generation error and cannot mark the corrupt output ready;
normal staging drop removes the private file.

Sixteen focused Linux actual-module tests pass, including valid bytes, truncated/
signature-only PDFs, incorrect/empty page counts, cycles, missing/malformed content,
rewinding and corrupt-publication cleanup, alongside existing containment tests.
An extracted unchanged production writer publishes the fake %PDF header fixture;
the same rejection assertion fails against it. Focused Clippy -D warnings, app/
harness formatting, locked app metadata and diff checks pass. The existing 3OS
confined-artifact harness now exercises these checks. No root dependency version or
Cargo.lock change; only the harness adds the already-used pinned parser dependency.

Native tests additionally exercise the actual Printpdf generator and a multi-page
English/Bangla/Russian report with tables/code through publication; those tests
were added but NOT run locally. Full app/native Windows/macOS CI is pending. No
physical desktop, independent rendered-page inspection or live provider smoke ran.

Do not close #347. These are structural checks, not PDF conformance certification,
semantic graphics/resource/font checks, proof of glyph shaping or unclipped table/
page layout. lopdf can repair some xref problems. Parsing/decompression uses memory
and remains synchronous; there is no new size/decompression quota or hostile-upload
sandbox. This gate is for local exports, not imported untrusted PDFs. DOCX ZIP/XML
validation, Word interoperability, independent rendering comparisons and async
render responsiveness remain follow-up work. Existing #394 root/cleanup and #353
filesystem/DB durability limits still apply. Do not claim all-green over known
RustSec and Windows dataset-junction failures.

Paused checkpoint: code/tests committed and published; no writer active after this
run. Check newer activity and latest native results before resuming or merging.
