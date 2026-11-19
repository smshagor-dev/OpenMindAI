# Image renderer safe publication

Previously image generation deleted an existing output before starting the renderer. A crash, timeout, invalid PNG or cancellation could destroy the old valid image.

The image renderer now writes to a unique sibling PNG, validates it, and publishes with an atomic no-clobber hard link. Drop removes the temporary file on errors and cancellation. Existing outputs are never overwritten.

Three Rust regression tests cover success, existing-output preservation and partial cleanup. Tests and actual renderer integration were not executed. Remaining: Windows filesystem validation, renderer CLI compatibility, stale stage cleanup after process crashes. Video generation is unchanged.
