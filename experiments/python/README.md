# Local personalization experiments

The optional pipeline trains a LoRA adapter from explicitly approved corrections,
evaluates held-out prompts, converts the adapter to GGUF, and activates it only
after a native generation probe. Training never overwrites the base model or
silently activates a candidate. Python is not required for normal desktop chat.

## Setup

Use Python 3.11+ and a separate environment. Install the CPU build of PyTorch,
then the training dependencies. The native service must already be built against
the pinned llama.cpp revision, with its runtime libraries available.

```sh
python -m venv experiments/python/.venv
# Activate this environment using your shell's normal activation command.
python -m pip install torch==2.14.0 --index-url https://download.pytorch.org/whl/cpu
python -m pip install transformers==4.57.6 peft==0.18.1 psutil==7.2.2 sentencepiece==0.2.2 protobuf==7.36.1
export PYTHONPATH=experiments/python
```

Training requires the corresponding local Hugging Face model snapshot in
safetensors format, its tokenizer/chat template, and the inference GGUF. All
loads use local files, disable remote code and keep the HF hub offline. The
operator must choose matching HF/GGUF weights; hashes bind subsequent use to
those exact files, but cannot prove that a separately supplied GGUF was produced
from that HF snapshot. Conversion uses llama.cpp commit
`7798007a29a90e3053e799394da48cf53a2f8e0f`.

## Approved feedback

Each JSONL record must have the following fields:

```json
{"approved":true,"profile_id":"local","user_input":"Explain promises briefly","assistant_output":"The previous answer","preferred_output":"The user's approved replacement","created_at":"2026-09-03T12:00:00Z"}
```

Use only corrections the user actually approved. A single file contains one
profile. At least 64 distinct training prompts and 12 held-out prompts are
required after deterministic splitting. Case/whitespace variants and repeated
corrections of the same prompt stay together; the latest correction is retained.
Do not put personal examples, adapters or model weights in the source repository.

```sh
python -m openmind_personalization train \
  --approved-feedback /absolute/feedback.jsonl \
  --profile-id local --model-id openmindai-nano \
  --base /absolute/local-hf-model --base-gguf /absolute/model.gguf \
  --output /absolute/profile/candidates --llama /absolute/llama.cpp
```

Default training uses CPU, two threads, 100 optimizer steps, rank 8 on q_proj and
v_proj, a 256-token example cap, 6 GiB process memory budget and a 20-minute total
deadline. It requires at least 4 GiB available system RAM and AC power. Initial
low CPU utilization is a readiness heuristic, not an OS user-idle detector.
The CLI monitors aggregate process memory/time and stops when other CPU work
resumes. Unknown process/power telemetry fails the CLI rather than bypassing
its gate. Resource sampling cannot guarantee avoidance of instantaneous OOM.

The candidate records held-out preferred-response cross-entropy with and without
the adapter, evaluation latency, counts, hashes and the training configuration.
Default acceptance requires at least 1% lower loss and no more than 1.5x baseline
evaluation time. This measures one local metric, not general intelligence,
factuality or production response quality. Review representative tasks before
using personal adapters. Rejected candidates remain inactive for inspection.

## Activation and rollback

Training prints a candidate.json path. Activate it separately:

```sh
python -m openmind_personalization activate \
  --candidate /absolute/profile/candidates/VERSION/candidate.json \
  --directory /absolute/profile/active --profile-id local --model-id openmindai-nano \
  --base-gguf /absolute/model.gguf --worker /absolute/openmind-native-worker
```

Activation validates profile/model identity, evaluation, base and adapter hashes,
then starts the actual Rust/CXX worker and requires a nonempty completed response.
Only then is the active pointer replaced atomically. The previous pointer is kept.
The activation directory is for one user profile. Treat it as private local data.

For desktop native chat, set `OPENMINDAI_NATIVE_ADAPTER_DIR` to that absolute
activation directory when launching the app. Pointer filenames are SHA256(model
ID) plus `.json`; the activation command prints the exact path. Native chat must
already be enabled. This does not enable native or Vulkan release defaults.
For the Go service, set a model registry entry's optional `personalization` field
to the printed activation file path. Its normal model path must match the hash.

```sh
python -m openmind_personalization rollback \
  --directory /absolute/profile/active --profile-id local --model-id openmindai-nano \
  --base-gguf /absolute/model.gguf --worker /absolute/openmind-native-worker
```

`disable` with the same arguments selects the unmodified base model explicitly.
Rollback revalidates an older adapter before restoring it. A pointer change takes
effect on the next request; in-flight requests finish with their original model.
Validation failures block personalized inference instead of silently falling
back to a backend that omits the adapter. Activation is local and is not signed
against a malicious local user who can edit the registry and model files.

## Validation and limitations

Run the stdlib tests with `python -m unittest discover -s experiments/python/tests`.
With optional dependencies, set `OPENMINDAI_TRAINING_INTEGRATION=1`,
`LLAMA_CPP_DIR`, and `OPENMINDAI_TEST_NATIVE_WORKER` to run the real synthetic
CPU training → GGUF conversion → native activation → rollback test. It also
checks that artifact corruption cannot change the active pointer.

This is an opt-in CLI pipeline. It does not add automatic conversation collection,
a desktop feedback/training screen, an idle scheduler or GPU/QLoRA training.
FP32 CPU training of a large model may exceed the user's RAM even when its
quantized GGUF runs comfortably. Qwen3/RX580 training and adapter quality require
separate evaluation on the actual machine and approved dataset.

The implementation uses [Hugging Face PEFT](https://huggingface.co/docs/peft/index)
and the [Transformers local model loader](https://huggingface.co/docs/transformers/en/main_classes/model).
