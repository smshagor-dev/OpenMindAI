from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True, slots=True)
class LearningPolicy:
    min_examples: int = 64
    min_holdout_examples: int = 12
    idle_only: bool = True
    max_training_minutes: int = 20
    min_free_ram_gb: float = 4.0
    require_ac_power: bool = True
    require_holdout_improvement: bool = True
    base_model_immutable: bool = True


@dataclass(frozen=True, slots=True)
class LearningDecision:
    ready: bool
    reason: str


def evaluate_training_readiness(
    policy: LearningPolicy,
    *,
    train_examples: int,
    holdout_examples: int,
    machine_idle: bool,
    free_ram_gb: float,
    on_ac_power: bool,
) -> LearningDecision:
    """Gate future adapter training before any ML framework is started."""

    if not policy.base_model_immutable:
        return LearningDecision(False, "base model mutation is not permitted")
    if train_examples < policy.min_examples:
        return LearningDecision(
            False,
            f"need at least {policy.min_examples} approved training examples",
        )
    if holdout_examples < policy.min_holdout_examples:
        return LearningDecision(
            False,
            f"need at least {policy.min_holdout_examples} holdout examples",
        )
    if policy.idle_only and not machine_idle:
        return LearningDecision(False, "machine is not idle")
    if free_ram_gb < policy.min_free_ram_gb:
        return LearningDecision(
            False,
            f"free RAM is below {policy.min_free_ram_gb:.1f} GiB",
        )
    if policy.require_ac_power and not on_ac_power:
        return LearningDecision(False, "training requires AC power")
    return LearningDecision(True, "adapter experiment may start")
