# Conversation-owned video cancellation (partial #363)

The active image job registry now also tracks video renderers under a conversation
ID. `cancel_generation(conversationId)` cancels only media jobs owned by that
conversation in addition to the existing chat inference cancellation. The
video render future uses Tokio cancellation selection; dropping the
stable-diffusion.cpp process future invokes its existing kill-on-drop process
ownership. Successful/failed artifact status is decided by the existing
generation result and cannot become Ready following token cancellation.

Image jobs remain cancellable by artifact ID, and the single-image renderer
semaphore is acquired only for images. Video does not accidentally occupy the
image-only queue.

Two registry regression tests verify that only the selected conversation's
image/video tokens are cancelled and that a finished job cannot cancel a
later job reusing the artifact identifier.

Voice synthesis is *not* registered for cancellation because its blocking
Kokoro worker keeps running after the awaiting future is dropped. A separate
cooperative TTS cancellation design and cleanup are still required. Video
GPU cleanup latency, process-tree termination, file write cancellation,
UI Stop affordance during video jobs, Windows/macOS/Linux smoke tests and
restart recovery all remain to be validated. Do not close #363.
