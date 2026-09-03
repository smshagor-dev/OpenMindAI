"""Offline LoRA experiments. Training artifacts never activate themselves."""
from __future__ import annotations

from dataclasses import asdict, dataclass
import hashlib
import json
import math
import os
from pathlib import Path
import subprocess
import sys
import time
import uuid

from .dataset import LearningExample, split_examples
from .policy import LearningPolicy, evaluate_training_readiness

LLAMA_PIN = "7798007a29a90e3053e799394da48cf53a2f8e0f"


def sha256(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def atomic_json(path: Path, value: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.{uuid.uuid4().hex}.tmp")
    try:
        with temporary.open("x", encoding="utf-8") as stream:
            os.chmod(temporary, 0o600)
            json.dump(value, stream, ensure_ascii=False, indent=2, allow_nan=False)
            stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


def read_json(path: Path) -> dict:
    if path.stat().st_size > 65536:
        raise ValueError("manifest exceeds 64 KiB")
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError("manifest must be an object")
    return value


def load_feedback(path: Path, profile: str) -> list[LearningExample]:
    if path.stat().st_size > 16 * 1024 * 1024:
        raise ValueError("feedback exceeds 16 MiB")
    examples = []
    with path.open(encoding="utf-8") as stream:
        for line in stream:
            item = json.loads(line)
            if item.pop("approved", None) is not True:
                raise ValueError("each example requires explicit approval")
            example = LearningExample(**item)
            example.validate()
            if example.profile_id != profile:
                raise ValueError("cross-profile feedback is not permitted")
            if max(map(len, (example.user_input, example.preferred_output))) > 32768:
                raise ValueError("feedback text is too large")
            examples.append(example)
    return examples


def source_fingerprint(directory: Path) -> str:
    files = sorted(p for p in directory.rglob("*") if p.is_file())
    if not files or any(p.is_symlink() for p in directory.rglob("*")):
        raise ValueError("base snapshot must contain regular local files without symlinks")
    digest = hashlib.sha256()
    for path in files:
        digest.update(str(path.relative_to(directory)).encode())
        digest.update(sha256(path).encode())
    return digest.hexdigest()


@dataclass(frozen=True)
class TrainingOptions:
    steps: int = 100
    learning_rate: float = 0.0002
    rank: int = 8
    max_length: int = 256
    threads: int = 2
    max_rss_gb: float = 6.0
    min_improvement: float = 0.01
    max_latency_ratio: float = 1.5

    def validate(self) -> None:
        if not (1 <= self.steps <= 2000 and 1 <= self.rank <= 32
                and 32 <= self.max_length <= 2048 and 1 <= self.threads <= 8
                and math.isfinite(self.learning_rate) and 0 < self.learning_rate <= 0.01
                and math.isfinite(self.max_rss_gb) and 0.25 <= self.max_rss_gb <= 32
                and 0 < self.min_improvement < 1 and 1 <= self.max_latency_ratio <= 3):
            raise ValueError("training options exceed supported limits")


def train_candidate(*, feedback: Path, profile_id: str, model_id: str, base: Path,
                    base_gguf: Path, output: Path, llama: Path,
                    policy: LearningPolicy = LearningPolicy(),
                    options: TrainingOptions = TrainingOptions()) -> Path:
    """Called in a supervised child process by the CLI, always using CPU training."""
    options.validate()
    if not profile_id or not model_id or len(profile_id) > 128 or len(model_id) > 128:
        raise ValueError("profile/model IDs must contain 1..128 characters")
    base, base_gguf, output, llama = (p.resolve() for p in (base, base_gguf, output, llama))
    if output == base or base in output.parents:
        raise ValueError("output must be outside the immutable base snapshot")
    revision = subprocess.check_output(["git", "-C", str(llama), "rev-parse", "HEAD"], text=True).strip()
    if revision != LLAMA_PIN:
        raise ValueError("conversion requires the pinned llama.cpp revision")
    examples = load_feedback(feedback, profile_id)
    train, holdout = split_examples(examples)
    import psutil
    battery = psutil.sensors_battery() if policy.require_ac_power else None
    decision = evaluate_training_readiness(
        policy, train_examples=len(train), holdout_examples=len(holdout),
        machine_idle=psutil.cpu_percent(interval=0.2) < 25,
        free_ram_gb=psutil.virtual_memory().available / 2**30,
        on_ac_power=battery is None or battery.power_plugged,
    )
    if not decision.ready:
        raise ValueError(decision.reason)
    os.environ.update(HF_HUB_OFFLINE="1", TRANSFORMERS_OFFLINE="1", TOKENIZERS_PARALLELISM="false")
    source_before, base_hash = source_fingerprint(base), sha256(base_gguf)
    run = output / uuid.uuid4().hex
    run.mkdir(parents=True, mode=0o700)
    deadline = time.monotonic() + policy.max_training_minutes * 60
    def resident_bytes() -> int:
        try:
            return psutil.Process().memory_info().rss
        except psutil.NoSuchProcess:
            # PID namespaces may omit this process from /proc. On Unix, the
            # kernel's own peak-RSS counter remains a conservative bound.
            import resource
            peak = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
            return int(peak if sys.platform == "darwin" else peak * 1024)


    def guard() -> None:
        battery = psutil.sensors_battery() if policy.require_ac_power else None
        if time.monotonic() >= deadline:
            raise TimeoutError("training deadline exceeded")
        if resident_bytes() > options.max_rss_gb * 2**30:
            raise MemoryError("training RSS budget exceeded")
        if psutil.virtual_memory().available < policy.min_free_ram_gb * 2**30:
            raise MemoryError("system memory reserve exhausted")
        if policy.require_ac_power and battery is not None and not battery.power_plugged:
            raise RuntimeError("AC power disconnected")

    import torch
    from peft import LoraConfig, TaskType, get_peft_model
    from transformers import AutoModelForCausalLM, AutoTokenizer
    torch.set_num_threads(options.threads)
    torch.manual_seed(17)
    tokenizer = AutoTokenizer.from_pretrained(base, local_files_only=True, trust_remote_code=False)
    model = AutoModelForCausalLM.from_pretrained(
        base, local_files_only=True, trust_remote_code=False, use_safetensors=True,
        torch_dtype=torch.float32,
    )
    guard()
    model = get_peft_model(model, LoraConfig(
        task_type=TaskType.CAUSAL_LM, r=options.rank, lora_alpha=options.rank * 2,
        target_modules=["q_proj", "v_proj"], lora_dropout=0.0, bias="none",
    ))

    def encode(example: LearningExample) -> dict:
        prefix = tokenizer.apply_chat_template(
            [{"role": "user", "content": example.user_input}], tokenize=True,
            add_generation_prompt=True,
        )
        answer = tokenizer.encode(example.preferred_output, add_special_tokens=False)
        if tokenizer.eos_token_id is not None:
            answer.append(tokenizer.eos_token_id)
        if not prefix or not answer or len(prefix) + len(answer) > options.max_length:
            raise ValueError("example exceeds token budget; shorten it explicitly")
        ids = prefix + answer
        return {"input_ids": torch.tensor([ids]), "attention_mask": torch.ones((1, len(ids)), dtype=torch.long),
                "labels": torch.tensor([[-100] * len(prefix) + answer])}

    train_batches, holdout_batches = [encode(e) for e in train], [encode(e) for e in holdout]

    def evaluate() -> tuple[float, float]:
        model.eval()
        losses = []
        with torch.no_grad():
            model(**holdout_batches[0])  # warm-up outside timing
            started = time.monotonic()
            for batch in holdout_batches:
                guard()
                loss = float(model(**batch).loss)
                if not math.isfinite(loss):
                    raise ValueError("non-finite holdout loss")
                losses.append(loss)
        return sum(losses) / len(losses), time.monotonic() - started

    with model.disable_adapter():
        baseline_loss, baseline_seconds = evaluate()
    optimizer = torch.optim.AdamW((p for p in model.parameters() if p.requires_grad), lr=options.learning_rate)
    model.train()
    for step in range(options.steps):
        guard()
        optimizer.zero_grad(set_to_none=True)
        loss = model(**train_batches[step % len(train_batches)]).loss
        if not torch.isfinite(loss):
            raise ValueError("non-finite training loss")
        loss.backward()
        torch.nn.utils.clip_grad_norm_((p for p in model.parameters() if p.requires_grad), 1.0)
        optimizer.step()
    candidate_loss, candidate_seconds = evaluate()
    evaluation = {
        "accepted": candidate_loss <= baseline_loss * (1 - options.min_improvement)
                    and candidate_seconds <= baseline_seconds * options.max_latency_ratio,
        "metric": "preferred_response_token_cross_entropy_on_local_HF_base",
        "baseline_loss": baseline_loss, "candidate_loss": candidate_loss,
        "baseline_seconds": baseline_seconds, "candidate_seconds": candidate_seconds,
        "holdout_examples": len(holdout), "train_examples": len(train),
    }
    adapter = run / "peft"
    model.save_pretrained(adapter, safe_serialization=True)
    converted = run / "adapter.gguf"
    guard()
    subprocess.run([sys.executable, str(llama / "convert_lora_to_gguf.py"),
                    "--base", str(base), "--outfile", str(converted), "--outtype", "f32", str(adapter)],
                   check=True, timeout=max(1, deadline - time.monotonic()),
                   env={**os.environ, "HF_HUB_OFFLINE": "1", "TRANSFORMERS_OFFLINE": "1"})
    guard()
    if converted.stat().st_size > 512 * 1024 * 1024:
        raise ValueError("adapter exceeds native artifact limit")
    if source_fingerprint(base) != source_before or sha256(base_gguf) != base_hash:
        raise ValueError("base model changed during experiment")
    candidate = run / "candidate.json"
    atomic_json(candidate, {
        "schema_version": 1, "profile_id": profile_id, "model_id": model_id,
        "base_sha256": base_hash, "source_fingerprint": source_before,
        "adapter_path": str(converted), "adapter_sha256": sha256(converted),
        "evaluation": evaluation, "options": asdict(options), "llama_commit": LLAMA_PIN,
        "probe_prompt": holdout[0].user_input,
    })
    return candidate
