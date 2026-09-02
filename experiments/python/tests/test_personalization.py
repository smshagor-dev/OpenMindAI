from openmind_personalization import (
    LearningExample,
    LearningPolicy,
    evaluate_training_readiness,
    split_examples,
)


def make_example(index: int) -> LearningExample:
    return LearningExample(
        user_input=f"question {index}",
        assistant_output=f"old answer {index}",
        preferred_output=f"preferred answer {index}",
        profile_id="local-profile",
        created_at=f"2026-09-02T18:{index % 60:02d}:00Z",
    )


def test_split_is_deterministic() -> None:
    examples = [make_example(index) for index in range(100)]
    first = split_examples(examples)
    second = split_examples(examples)
    assert first == second
    assert first[0]
    assert first[1]


def test_learning_waits_for_enough_explicit_feedback() -> None:
    decision = evaluate_training_readiness(
        LearningPolicy(),
        train_examples=10,
        holdout_examples=20,
        machine_idle=True,
        free_ram_gb=8.0,
        on_ac_power=True,
    )
    assert not decision.ready
    assert "approved training examples" in decision.reason


def test_learning_can_start_only_after_resource_and_data_gates() -> None:
    decision = evaluate_training_readiness(
        LearningPolicy(),
        train_examples=80,
        holdout_examples=20,
        machine_idle=True,
        free_ram_gb=8.0,
        on_ac_power=True,
    )
    assert decision.ready
