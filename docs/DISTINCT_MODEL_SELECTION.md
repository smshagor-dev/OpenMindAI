# Selecting distinct installed models

The chat selector retains enabled models by stable registry ID, rather than
choosing a single record per friendly display name. Options show quantization,
size and filename; identical details additionally display the full model ID at
the beginning. The active trigger includes details when its friendly name is
ambiguous. Tooltips expose the full selection label/path. Sorting is by friendly
name, details and ID, so changing selection does not hide a sibling model.

The unavailable/restored model behavior introduced in #487 remains covered.
Unknown custom GGUFs, Windows/Unix filenames, two quantizations of a known family,
identical filenames and reordered records are exercised with the actual React
component in a DOM. Each distinct option sends its original registry ID.

Validation: npm run test:model-inventory (4 tests including 3 subtests), all 96
frontend tests, npm run lint, npm run build, and git diff --check passed locally.
The new duplicate-model regressions fail against the previous production selector
before the fix. Existing selector styling is reused; no real GGUF import/load,
physical desktop click-through or visual layout verification is claimed. Runtime
backend selection and friendly-name mapping are unchanged. Native frontend/build
and CodeQL checks must finish before merge; existing Rust audit and Windows
junction fixture failures remain separate baseline issues.
