# Procedural soundscape disclosure (partial #361)

The soundscape command is a deterministic WAV synthesizer. It does not run
Stable Audio Open, even if that model is installed. The command no longer
consults the installed-model flag to imply model-backed generation. It
explicitly identifies procedural output in the artifact filename and rejects
unsupported arbitrary prompts rather than silently generating unrelated music.

The backend allows explicit rain, ocean, wind, notification/alert and simple
music/ambient presets. The validator runs before any artifact row or staging
file is created, and again at the WAV renderer boundary. Existing WAV format
validation remains. Rust unit tests cover accepted presets and unsupported
requests leaving no file.

Remaining: real Stable Audio Open inference backend, explicit model/procedural
selector in the UI, provenance metadata, and physical playback quality tests.
This is a truthful-fallback safety change, not model-inference support.
