"""Bounded streaming evidence reuses the existing collector and stats owner."""
from typing import Literal

from pydantic import Field, PositiveInt

from .configschema import Strict


class StreamCaseEvidence(Strict):
    sample_key: Literal["keystrokes", "resize_samples", "runs", "transcript_samples", "events_ping_samples", "event_transitions"]
    metrics: tuple[str, ...] = Field(min_length=1)


class StreamEndpointEvidence(Strict):
    collector: Literal["stream"]
    target_scope: Literal["gateway", "both"]
    reason: str = Field(min_length=20)
    cases: dict[str, StreamCaseEvidence] = Field(min_length=1)


class StreamConfig(Strict):
    runs: PositiveInt
    quick_runs: PositiveInt
    keystrokes: PositiveInt
    quick_keystrokes: PositiveInt
    resize_samples: Literal[20]
    terminal_bytes: PositiveInt
    exec_bytes: PositiveInt
    marker_timeout_seconds: PositiveInt
    transcript_samples: PositiveInt
    quick_transcript_samples: PositiveInt
    transcript_tail_lines: PositiveInt
    transcript_max_bytes: PositiveInt
    transcript_rotation_min_bytes: PositiveInt
    transcript_rotated_path: str = Field(min_length=1)
    events_ping_samples: PositiveInt
    quick_events_ping_samples: PositiveInt
    event_transitions: Literal[1]
    events_max_frame_bytes: PositiveInt
    events_max_frames: PositiveInt
    events_timeout_seconds: PositiveInt
    events_open_timeout_seconds: PositiveInt
    events_close_timeout_seconds: PositiveInt
    mutation_timeout_seconds: PositiveInt
