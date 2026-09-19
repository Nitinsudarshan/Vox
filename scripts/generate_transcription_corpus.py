#!/usr/bin/env python3
"""
CLI entrypoint to generate the fixed Vox long-form transcription benchmark corpus.
"""
import asyncio
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(REPO_ROOT / "tests" / "transcription" / "generator"))

from generate_corpus import generate_all

if __name__ == "__main__":
    asyncio.run(generate_all())
