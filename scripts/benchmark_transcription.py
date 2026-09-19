#!/usr/bin/env python3
"""
CLI entrypoint to run the Vox long-form transcription benchmark.
"""
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(REPO_ROOT / "tests" / "transcription" / "runner"))

from run_benchmark import main

if __name__ == "__main__":
    main()
