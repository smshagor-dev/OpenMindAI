# DOCX export integrity: partial #347

StagedOutput validates DOCX packages from the same opened source before publication.
The ZIP central directory and every exposed entry are read and CRC-checked. A second
local-header pass rejects duplicate local entries and local/central name/size/CRC
mismatches instead of trusting zip's name-deduplicated central index. No files are
extracted. Unsafe paths, symlink entries, encryption and ZIP prefixes are rejected.
All .xml/.rels parts must be well-formed UTF-8 XML without DTDs. Required content
Types, package Relationships and Word document/body namespaces are checked; exactly
one correctly typed internal relationship must target word/document.xml. The main
content-type override must match the generated Word document type.

Validation limits are explicit: archive bytes and total uncompressed entry bytes
must each fit 128 MiB (each ZIP pass enforces the expansion budget); each XML parse
allows at most 1,000,000 nodes. Larger exports now fail with a validation-limit
message. ZIP parsing still allocates its own index before our entry loop; these
limits are not a general hostile-input sandbox. The two ZIP passes cost extra CPU.
The gate targets the application's seekable UTF-8/transitional OOXML output;
streaming data-descriptor ZIPs, strict OOXML or other encodings are not guaranteed.

Twenty-four focused production-module Linux tests passed, including real deflated
UTF-8 packages, fake/truncated ZIPs, missing parts, wrong targets, malformed XML,
namespace/body/DTD checks, correct XML with wrong CRC, duplicate local names,
traversal, oversized sparse input and corrupt-publication/staging cleanup, plus
prior PDF/containment cases. The old extracted production writer publishes the fake
PK header fixture and fails the identical rejection assertion. Strict focused Clippy,
app/harness formatting, locked root metadata and diff checks passed.

An actual generator test for English/Bangla/Russian tables through publication was
added to native tests but NOT run locally. Three-OS focused/full native CI must
confirm actual generator compatibility. No Microsoft Word/LibreOffice/physical
desktop or rendered-page comparison was performed. Well-formed package/XML checks
are not full OOXML schema validation, style/relationship graph correctness, page
layout/glyph/Word interoperability or responsiveness proof. No async rendering fix.
Keep #347 open for those follow-ups. Existing #394/durability and RustSec/Windows
junction caveats remain; no all-green or limitation-free claim.

roxmltree 0.21.1 is now a direct dependency, pinned to the already locked transitive
version; the root lockfile changes only the application's dependency edge. No
existing package versions change. The test harness adds the existing pinned ZIP9
and parser dependencies. No schema/frontend changes.

Paused checkpoint: code/tests committed and published; no writer active after this
run. Check newer branch activity and current native checks before resuming/merging.
