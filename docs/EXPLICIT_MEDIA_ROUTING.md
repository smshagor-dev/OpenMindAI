# Explicit media command routing

A bounded command heuristic now recognizes requested image/video/voice/sound
outputs before attachment analysis and generic search/document topic keywords.
English commands use their first recognized output noun, so a voiceover *for an
image* remains voice, and a video *for a report* remains video. Report/document
and code/tutorial nouns do not enter this media fast path. Bengali commands and
Russian leading imperatives have corresponding media noun recognition; Bengali
possessive topic nouns are skipped when finding the requested output.

Existing image intent/idiom checks remain in use. Analysis-only attachment inputs,
research, search and document fallbacks retain their existing ordering. Input is
trimmed, case-folded and NFKC-normalized before routing. Renderer selection,
Cloud/Local privacy policy, authorization and attachment ingestion are unchanged.

Validation: npm run test:chat-routing passed two table-driven tests: 17 media
requests in English/Bengali/Russian, each with no attachment, image, PDF and both,
plus 13 analysis/search/research/document fallback cases. The same suite fails on
the old production router (news image request incorrectly selects search).
All 129 frontend tests passed; lint completed with zero errors and two existing
hook-dependency warnings in StudioHub.tsx; TypeScript/Vite build and diff checks
passed. No Rust source/dependency changes were made.

This remains deterministic routing, not a complete language/negation parser.
Multiple explicit output requests still select one mode; a research-then-render
workflow is not implemented here. Physical renderer/provider execution and desktop
visual interaction were not tested. CI remains the integration gate; existing
RustSec and Windows dataset-junction failures must be reported separately.
