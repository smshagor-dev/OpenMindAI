import unittest

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


class PersonalizationContractTests(unittest.TestCase):
    def test_split_is_deterministic(self) -> None:
        examples = [make_example(index) for index in range(100)]
        first = split_examples(examples)
        second = split_examples(examples)
        self.assertEqual(first, second)
        self.assertTrue(first[0])
        self.assertTrue(first[1])

    def test_learning_waits_for_enough_explicit_feedback(self) -> None:
        decision = evaluate_training_readiness(
            LearningPolicy(),
            train_examples=10,
            holdout_examples=20,
            machine_idle=True,
            free_ram_gb=8.0,
            on_ac_power=True,
        )
        self.assertFalse(decision.ready)
        self.assertIn("approved training examples", decision.reason)

    def test_learning_can_start_only_after_resource_and_data_gates(self) -> None:
        decision = evaluate_training_readiness(
            LearningPolicy(),
            train_examples=80,
            holdout_examples=20,
            machine_idle=True,
            free_ram_gb=8.0,
            on_ac_power=True,
        )
        self.assertTrue(decision.ready)


if __name__ == "__main__":
    unittest.main()
