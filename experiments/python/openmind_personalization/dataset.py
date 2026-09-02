from __future__ import annotations

from dataclasses import dataclass
import hashlib
from typing import Iterable


@dataclass(frozen=True, slots=True)
class LearningExample:
    """A user-approved correction or preference example.

    `preferred_output` must come from explicit user feedback/correction. Raw conversations
    are intentionally not treated as training examples by this experiment layer.
    """

    user_input: str
    assistant_output: str
    preferred_output: str
    profile_id: str
    created_at: str

    def validate(self) -> None:
        for name, value in (
            ("user_input", self.user_input),
            ("assistant_output", self.assistant_output),
            ("preferred_output", self.preferred_output),
            ("profile_id", self.profile_id),
            ("created_at", self.created_at),
        ):
            if not value.strip():
                raise ValueError(f"{name} must not be empty")


def split_examples(
    examples: Iterable[LearningExample],
    *,
    holdout_ratio: float = 0.2,
) -> tuple[list[LearningExample], list[LearningExample]]:
    """Create a stable train/holdout split without requiring third-party packages."""

    if not 0.05 <= holdout_ratio <= 0.5:
        raise ValueError("holdout_ratio must be between 0.05 and 0.5")

    train: list[LearningExample] = []
    holdout: list[LearningExample] = []
    threshold = int(holdout_ratio * 10_000)

    for example in examples:
        example.validate()
        digest = hashlib.sha256(
            "\x1f".join(
                [
                    example.profile_id,
                    example.user_input,
                    example.preferred_output,
                    example.created_at,
                ]
            ).encode("utf-8")
        ).digest()
        bucket = int.from_bytes(digest[:4], "big") % 10_000
        (holdout if bucket < threshold else train).append(example)

    return train, holdout
