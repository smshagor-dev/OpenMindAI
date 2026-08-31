// iOS intentionally reuses the same embedded llama.cpp inference engine as Android.
// The dependency backend differs by target (Metal on iOS, NDK/shared libc++ on Android),
// while model selection, prompt rendering, streaming, cancellation, and lifecycle
// behavior remain identical.
include!("mobile_inference.rs");
