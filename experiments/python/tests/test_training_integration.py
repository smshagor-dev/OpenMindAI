"""Optional real CPU training, GGUF conversion and native activation fixture."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import queue
import threading
import unittest

from openmind_personalization.activation import activate, active_path, rollback
from openmind_personalization.pipeline import TrainingOptions, atomic_json, read_json, sha256, train_candidate
from openmind_personalization.policy import LearningPolicy


@unittest.skipUnless(os.environ.get("OPENMINDAI_TRAINING_INTEGRATION") == "1", "optional ML dependencies")
class TrainingIntegration(unittest.TestCase):
    def test_real_training_conversion_activation_rollback(self):
        import sentencepiece as spm
        import torch
        from transformers import LlamaConfig, LlamaForCausalLM, LlamaTokenizer
        llama = Path(os.environ["LLAMA_CPP_DIR"]).resolve()
        worker = Path(os.environ["OPENMINDAI_TEST_NATIVE_WORKER"]).resolve()
        with tempfile.TemporaryDirectory(prefix="openmind-personalization-test-") as temporary:
            root = Path(temporary)
            base = root / "base"
            base.mkdir()
            corpus = root / "corpus.txt"
            corpus.write_text("\n".join(f"question {i} color blue blue answer" for i in range(200)))
            spm.SentencePieceTrainer.train(input=str(corpus), model_prefix=str(base / "tokenizer"),
                                          vocab_size=384, byte_fallback=True, model_type="bpe", bos_id=1, eos_id=2,
                                          unk_id=0, pad_id=3, minloglevel=2)
            tokenizer = LlamaTokenizer(vocab_file=str(base / "tokenizer.model"))
            tokenizer.chat_template = "{{ bos_token }}{% for message in messages %}{% if message['role'] == 'user' %}{{ '[INST] ' + message['content'] + ' [/INST]' }}{% elif message['role'] == 'assistant' %}{{ ' ' + message['content'] + eos_token }}{% endif %}{% endfor %}"
            tokenizer.save_pretrained(base)
            torch.manual_seed(17)
            model = LlamaForCausalLM(LlamaConfig(
                vocab_size=384, hidden_size=32, intermediate_size=64, num_hidden_layers=2,
                num_attention_heads=4, num_key_value_heads=2, max_position_embeddings=1024,
                bos_token_id=1, eos_token_id=2, pad_token_id=3,
            ))
            model.save_pretrained(base, safe_serialization=True)
            gguf = root / "base.gguf"
            subprocess.run([sys.executable, str(llama / "convert_hf_to_gguf.py"), str(base),
                            "--outfile", str(gguf), "--outtype", "f32"], check=True)
            base_hash = sha256(gguf)
            feedback = root / "feedback.jsonl"
            feedback.write_text("\n".join(json.dumps({
                "approved": True, "profile_id": "test", "user_input": f"question {i} color",
                "assistant_output": "red", "preferred_output": "blue blue",
                "created_at": f"2026-09-03T00:{i % 60:02d}:00Z",
            }) for i in range(200)))
            # Exercise production process supervision when /proc exposes this
            # PID. Restricted PID namespaces still exercise real training below.
            import psutil
            from openmind_personalization.__main__ import supervised_training
            def run_training(**arguments):
                try:
                    psutil.Process().memory_info()
                except psutil.NoSuchProcess:
                    return train_candidate(**arguments)
                return supervised_training(arguments)
            candidate = run_training(
                feedback=feedback, profile_id="test", model_id="nano", base=base,
                base_gguf=gguf, output=root / "candidates", llama=llama,
                policy=LearningPolicy(idle_only=False, min_free_ram_gb=0.1, require_ac_power=False),
                options=TrainingOptions(steps=80, learning_rate=0.005, rank=4, max_length=128,
                                        threads=1, min_improvement=0.001, max_latency_ratio=3),
            )
            evaluation = read_json(candidate)["evaluation"]
            self.assertLess(evaluation["candidate_loss"], evaluation["baseline_loss"])
            self.assertTrue(evaluation["accepted"], evaluation)
            self.assertEqual(base_hash, sha256(gguf))
            print("Synthetic adapter evaluation:", json.dumps(evaluation, sort_keys=True))
            active = active_path(root / "active", "nano")
            atomic_json(active, {"schema_version": 1, "profile_id": "test", "model_id": "nano", "candidate": None})
            registry = root / "models.json"
            atomic_json(registry, {"nano": {"path": str(gguf), "context_size": 512, "personalization": str(active)}})
            process = subprocess.Popen([str(worker), "--models", str(registry)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
            self.addCleanup(lambda: process.kill() if process.poll() is None else None)
            received = queue.Queue()
            def consume():
                for line in process.stdout:
                    received.put(json.loads(line))
            reader = threading.Thread(target=consume, daemon=True)
            reader.start()
            self.assertEqual(received.get(timeout=10)["type"], "ready")
            def generate(request_id):
                request = {"id": request_id, "model": "nano", "temperature": 0, "max_tokens": 16,
                           "messages": [{"role": "user", "content": "question 9 color"}]}
                process.stdin.write(json.dumps(request) + "\n"); process.stdin.flush()
                text = ""
                while True:
                    event = received.get(timeout=30)
                    self.assertEqual(event["id"], request_id)
                    self.assertNotEqual(event["type"], "error", event)
                    if event["type"] == "done": return text
                    text += event.get("text", "")
            original_output = generate(1)
            active = activate(candidate, directory=root / "active", profile_id="test", model_id="nano",
                              base_gguf=gguf, worker=worker)
            self.assertEqual(read_json(active)["candidate"], str(candidate))
            personalized_output = generate(2)
            self.assertTrue(personalized_output)
            self.assertNotEqual(original_output, personalized_output)
            rollback(directory=root / "active", profile_id="test", model_id="nano", base_gguf=gguf, worker=worker)
            self.assertIsNone(read_json(active)["candidate"])
            self.assertEqual(generate(3), original_output)
            process.stdin.close()
            process.wait(timeout=10)
            reader.join(timeout=2)
            process.stdout.close()
            # Corrupt artifacts cannot become active, and rejection preserves the pointer.
            adapter = Path(read_json(candidate)["adapter_path"])
            with adapter.open("ab") as stream:
                stream.write(b"corrupt")
            with self.assertRaises(ValueError):
                activate(candidate, directory=root / "active", profile_id="test", model_id="nano",
                         base_gguf=gguf, worker=worker)
            self.assertIsNone(read_json(active)["candidate"])
