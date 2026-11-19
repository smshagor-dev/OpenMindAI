# DOCX relationship security gate — partial #347

DOCX integrity validation now parses every .rels entry (including nested Word
relationships), not only the package root relationship. It rejects external
targets and TargetMode=External, traversal, absolute and URL-style targets,
malformed relationship nodes, and unsafe nested relationship packages.
Normal relative internal relationships remain accepted.

Three Rust regression tests cover external package links, nested remote
resources and a valid internal styles relationship. These tests are added but
have not been executed; cargo compilation and genuine Word/LibreOffice
rendering validation remain necessary. This does not complete #347.
