from __future__ import annotations

import argparse
import multiprocessing
from pathlib import Path
import queue
import time

from .activation import activate, rollback
from .pipeline import TrainingOptions, train_candidate
from .policy import LearningPolicy


def training_child(arguments: dict, results) -> None:
    try:
        results.put((True, str(train_candidate(**arguments))))
    except Exception as error:
        results.put((False, f"{type(error).__name__}: {error}"))


def supervised_training(arguments: dict) -> Path:
    import psutil
    context = multiprocessing.get_context("spawn")
    results = context.Queue(maxsize=1)
    child = context.Process(target=training_child, args=(arguments, results))
    child.start()
    watched = None
    deadline = time.monotonic() + arguments["policy"].max_training_minutes * 60
    try:
        watched = psutil.Process(child.pid)
        psutil.cpu_percent(interval=None)
        watched.cpu_percent(interval=None)
        while child.is_alive():
            child.join(0.5)
            if not child.is_alive():
                break
            processes = [watched, *watched.children(recursive=True)]
            rss = sum(p.memory_info().rss for p in processes if p.is_running())
            other_cpu = psutil.cpu_percent(interval=None) - sum(p.cpu_percent(interval=None) for p in processes if p.is_running()) / max(psutil.cpu_count() or 1, 1)
            if arguments["policy"].idle_only and other_cpu >= 25:
                raise RuntimeError("other CPU work resumed; training stopped")
            if time.monotonic() >= deadline or rss > arguments["options"].max_rss_gb * 2**30:
                raise RuntimeError("training exceeded its process time/memory budget")
        try:
            success, value = results.get(timeout=2)
        except queue.Empty as error:
            raise RuntimeError(f"training process exited without a result ({child.exitcode})") from error
        if not success or child.exitcode != 0:
            raise RuntimeError(value)
        return Path(value)
    finally:
        if child.is_alive():
            for process in watched.children(recursive=True) if watched is not None else []:
                try:
                    process.kill()
                except psutil.NoSuchProcess:
                    pass
            child.kill()
        child.join()
        results.close()


def main() -> None:
    parser = argparse.ArgumentParser(description="Explicit local personalization experiments")
    sub = parser.add_subparsers(dest="command", required=True)
    train = sub.add_parser("train")
    for name in ("approved-feedback", "base", "base-gguf", "output", "llama"):
        train.add_argument("--" + name, type=Path, required=True)
    train.add_argument("--profile-id", required=True)
    train.add_argument("--model-id", required=True)
    train.add_argument("--steps", type=int, default=100)
    train.add_argument("--max-rss-gb", type=float, default=6)
    for name in ("activate", "rollback", "disable"):
        command = sub.add_parser(name)
        for field in ("directory", "base-gguf", "worker"):
            command.add_argument("--" + field, type=Path, required=True)
        command.add_argument("--profile-id", required=True)
        command.add_argument("--model-id", required=True)
        if name == "activate":
            command.add_argument("--candidate", type=Path, required=True)
    args = vars(parser.parse_args())
    command = args.pop("command")
    if command == "train":
        args["feedback"] = args.pop("approved_feedback")
        args["options"] = TrainingOptions(steps=args.pop("steps"), max_rss_gb=args.pop("max_rss_gb"))
        args["policy"] = LearningPolicy()
        print(supervised_training(args))
    elif command == "activate":
        print(activate(**args))
    else:
        print(rollback(**args, disable=command == "disable"))


if __name__ == "__main__":
    main()
