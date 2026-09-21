"""
Public Speech Dataset Abstraction and Normalized Schema for Vox.

Provides standardized data structures for importing, representing, and evaluating
external speech corpora (e.g. Mozilla Common Voice, Google FLEURS) within the Vox
transcription benchmark suite.
"""

from __future__ import annotations

import dataclasses
import hashlib
import json
import os
from pathlib import Path
from typing import Any, Dict, List, Optional


@dataclasses.dataclass
class PublicSpeechSample:
    """A single evaluation utterance/clip in a public speech dataset."""
    sample_id: str
    dataset_name: str
    dataset_version: str
    language: str  # BCP-47 language tag (e.g. 'en', 'hi', 'ta')
    split: str     # 'test', 'dev', 'train', 'spontaneous'
    audio_path: str  # Relative or absolute path to 16kHz mono WAV file
    reference_transcript: str
    source_url: str
    license: str
    speaker_id: Optional[str] = None
    duration_seconds: Optional[float] = None
    sample_rate: Optional[int] = 16000
    audio_checksum_sha256: Optional[str] = None
    metadata: Dict[str, Any] = dataclasses.field(default_factory=dict)

    def to_dict(self) -> Dict[str, Any]:
        return dataclasses.asdict(self)

    @classmethod
    def from_dict(cls, data: Dict[str, Any]) -> PublicSpeechSample:
        return cls(
            sample_id=str(data["sample_id"]),
            dataset_name=str(data["dataset_name"]),
            dataset_version=str(data["dataset_version"]),
            language=str(data["language"]),
            split=str(data.get("split", "test")),
            audio_path=str(data["audio_path"]),
            reference_transcript=str(data["reference_transcript"]),
            source_url=str(data.get("source_url", "")),
            license=str(data.get("license", "")),
            speaker_id=data.get("speaker_id"),
            duration_seconds=float(data["duration_seconds"]) if data.get("duration_seconds") is not None else None,
            sample_rate=int(data.get("sample_rate", 16000)) if data.get("sample_rate") is not None else 16000,
            audio_checksum_sha256=data.get("audio_checksum_sha256"),
            metadata=dict(data.get("metadata", {}))
        )

    def compute_audio_sha256(self, base_dir: Optional[Path] = None) -> Optional[str]:
        """Computes and updates the sha256 checksum of the audio file if it exists."""
        p = Path(self.audio_path)
        if not p.is_absolute() and base_dir:
            p = base_dir / p
        if not p.is_file():
            return None
        hasher = hashlib.sha256()
        with open(p, "rb") as f:
            for chunk in iter(lambda: f.read(65536), b""):
                hasher.update(chunk)
        self.audio_checksum_sha256 = hasher.hexdigest()
        return self.audio_checksum_sha256


@dataclasses.dataclass
class PublicSpeechDataset:
    """A collection of samples representing a specific public speech dataset manifest."""
    dataset_name: str
    dataset_version: str
    language: str
    split: str
    source_url: str
    license: str
    manifest_version: str = "v1"
    samples: List[PublicSpeechSample] = dataclasses.field(default_factory=list)
    description: str = ""
    sampling_methodology: str = ""

    @property
    def total_samples(self) -> int:
        return len(self.samples)

    @property
    def total_duration_seconds(self) -> float:
        return sum(s.duration_seconds or 0.0 for s in self.samples)

    @property
    def total_reference_words(self) -> int:
        return sum(len(s.reference_transcript.split()) for s in self.samples)

    @property
    def unique_speakers_count(self) -> int:
        speakers = {s.speaker_id for s in self.samples if s.speaker_id}
        return len(speakers)

    def to_jsonl(self, output_path: Path) -> None:
        """Writes samples to a JSONL manifest file."""
        output_path.parent.mkdir(parents=True, exist_ok=True)
        with open(output_path, "w", encoding="utf-8") as f:
            for sample in self.samples:
                f.write(json.dumps(sample.to_dict(), ensure_ascii=False) + "\n")

    @classmethod
    def from_jsonl(
        cls,
        jsonl_path: Path,
        dataset_name: str = "",
        dataset_version: str = "",
        language: str = "",
        split: str = "test",
        source_url: str = "",
        license: str = "",
        manifest_version: str = "v1",
        description: str = "",
        sampling_methodology: str = ""
    ) -> PublicSpeechDataset:
        """Loads a dataset from a normalized JSONL manifest."""
        samples: List[PublicSpeechSample] = []
        if not jsonl_path.is_file():
            raise FileNotFoundError(f"Manifest file not found: {jsonl_path}")

        with open(jsonl_path, "r", encoding="utf-8") as f:
            for line_idx, line in enumerate(f):
                line = line.strip()
                if not line or line.startswith("#"):
                    continue
                try:
                    data = json.loads(line)
                    sample = PublicSpeechSample.from_dict(data)
                    samples.append(sample)
                except Exception as e:
                    raise ValueError(f"Error parsing line {line_idx + 1} of {jsonl_path}: {e}")

        if samples and not dataset_name:
            dataset_name = samples[0].dataset_name
        if samples and not dataset_version:
            dataset_version = samples[0].dataset_version
        if samples and not language:
            language = samples[0].language
        if samples and not split:
            split = samples[0].split
        if samples and not source_url:
            source_url = samples[0].source_url
        if samples and not license:
            license = samples[0].license

        return cls(
            dataset_name=dataset_name,
            dataset_version=dataset_version,
            language=language,
            split=split,
            source_url=source_url,
            license=license,
            manifest_version=manifest_version,
            samples=samples,
            description=description,
            sampling_methodology=sampling_methodology,
        )
