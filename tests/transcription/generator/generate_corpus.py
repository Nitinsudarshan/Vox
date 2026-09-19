"""
Automated Long-Form Test Corpus Generator for Vox Meeting Transcription Pipeline.
Generates corpus-v1: 16 kHz mono 16-bit PCM WAV, ground truth reference texts, and structured metadata.
"""

import array
import asyncio
import datetime
import json
import math
import os
import random
import shutil
import sys
import wave
from pathlib import Path

import edge_tts
import miniaudio

# Add generator directory to path
SCRIPT_DIR = Path(__file__).resolve().parent
REPO_ROOT = SCRIPT_DIR.parents[2]
sys.path.insert(0, str(SCRIPT_DIR))

from corpus_definitions import CORPUS_VERSION, TEST_CASES, validate_corpus

CORPUS_DIR = REPO_ROOT / "tests" / "transcription" / CORPUS_VERSION
REFERENCES_DIR = REPO_ROOT / "tests" / "transcription" / "references"
FIXTURES_DIR = REPO_ROOT / "fixtures" / "audio"

def add_ambient_noise(samples: list[float], snr_db: float = 22.0) -> list[float]:
    """Adds realistic pink/ambient room noise to audio samples at target SNR."""
    # Compute RMS of signal
    sum_sq = sum(s * s for s in samples)
    if not sum_sq:
        return samples
    signal_rms = math.sqrt(sum_sq / len(samples))
    target_noise_rms = signal_rms / (10.0 ** (snr_db / 20.0))

    # Generate 1st-order filtered pink-ish room noise
    b0, b1, b2 = 0.0, 0.0, 0.0
    random.seed(42)  # Deterministic seed
    noisy_samples = []
    
    for s in samples:
        white = random.gauss(0.0, 1.0)
        b0 = 0.99765 * b0 + white * 0.0990460
        b1 = 0.96300 * b1 + white * 0.2965164
        b2 = 0.57000 * b2 + white * 1.0526913
        pink = b0 + b1 + b2 + white * 0.1848
        
        # Scale to target noise RMS
        sample_with_noise = s + (pink * 0.35 * target_noise_rms)
        # Soft clamp
        sample_with_noise = max(-1.0, min(1.0, sample_with_noise))
        noisy_samples.append(sample_with_noise)
        
    return noisy_samples

async def synthesize_case(case: dict) -> dict:
    case_id = case["id"]
    lang = case["language"]
    voice = case["voice"]
    rate = case["rate"]
    text = case["text"].strip()
    words = text.split()
    word_count = len(words)
    char_count = len(text)
    
    lang_dir = CORPUS_DIR / lang
    lang_dir.mkdir(parents=True, exist_ok=True)
    REFERENCES_DIR.mkdir(parents=True, exist_ok=True)
    FIXTURES_DIR.mkdir(parents=True, exist_ok=True)
    
    wav_path = lang_dir / f"{case_id}.wav"
    txt_path = lang_dir / f"{case_id}.txt"
    json_path = lang_dir / f"{case_id}.json"
    ref_txt_path = REFERENCES_DIR / f"{case_id}.txt"
    fixture_wav_path = FIXTURES_DIR / f"{case_id}.wav"
    
    print(f"\n[SYNTHESIZING] {case_id} ({lang}) -> words={word_count}, voice={voice}, rate={rate}...")
    
    # 1. Write reference text
    with open(txt_path, "w", encoding="utf-8") as f:
        f.write(text)
    with open(ref_txt_path, "w", encoding="utf-8") as f:
        f.write(text)

    # 2. Synthesize via edge-tts
    comm = edge_tts.Communicate(text, voice, rate=rate)
    raw_mp3 = b""
    async for chunk in comm.stream():
        if chunk["type"] == "audio":
            raw_mp3 += chunk["data"]
            
    if not raw_mp3:
        raise RuntimeError(f"Failed to synthesize audio for {case_id}: empty buffer received!")
        
    # 3. Decode to 16 kHz mono float/pcm
    decoded = miniaudio.decode(raw_mp3, nchannels=1, sample_rate=16000)
    samples = list(decoded.samples)
    
    # 4. Optional background noise
    if case.get("add_noise", False):
        print(f"  Adding moderate ambient room noise (SNR=22dB) to {case_id}...")
        samples = add_ambient_noise(samples, snr_db=22.0)
        
    duration_sec = len(samples) / 16000.0
    
    # 5. Write 16-bit PCM WAV using standard library wave + array
    int16_arr = array.array('h', (int(max(-1.0, min(1.0, s)) * 32767.0) for s in samples))
    with wave.open(str(wav_path), "wb") as wf:
        wf.setnchannels(1)
        wf.setsampwidth(2)
        wf.setframerate(16000)
        wf.writeframes(int16_arr.tobytes())
        
    shutil.copy2(str(wav_path), str(fixture_wav_path))
    
    file_size_bytes = os.path.getsize(wav_path)
    
    # 6. Structured metadata
    meta = {
        "id": case_id,
        "corpus_version": CORPUS_VERSION,
        "language": lang,
        "category": case["category"],
        "title": case["title"],
        "description": case["description"],
        "voice": voice,
        "rate": rate,
        "word_count": word_count,
        "char_count": char_count,
        "duration_seconds": round(duration_sec, 2),
        "sample_rate": 16000,
        "channels": 1,
        "format": "pcm_s16le",
        "file_size_bytes": file_size_bytes,
        "tts_engine": "edge-tts-7.2.8",
        "added_ambient_noise": case.get("add_noise", False),
        "generated_at": datetime.datetime.now(datetime.timezone.utc).isoformat()
    }
    
    with open(json_path, "w", encoding="utf-8") as f:
        json.dump(meta, f, indent=2)
        
    print(f"  Completed {case_id}: {duration_sec:.1f}s audio ({file_size_bytes / 1024 / 1024:.2f} MB), {word_count} words.")
    return meta

async def generate_all():
    print(f"Starting automated corpus generation for {CORPUS_VERSION}...")
    validate_corpus()
    
    manifest = {
        "version": CORPUS_VERSION,
        "created_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "cases_count": len(TEST_CASES),
        "cases": []
    }
    
    total_words = 0
    total_duration = 0.0
    
    for case in TEST_CASES:
        meta = await synthesize_case(case)
        manifest["cases"].append(meta)
        total_words += meta["word_count"]
        total_duration += meta["duration_seconds"]
        
    manifest["total_words"] = total_words
    manifest["total_duration_seconds"] = round(total_duration, 2)
    manifest["total_duration_minutes"] = round(total_duration / 60.0, 2)
    
    manifest_path = CORPUS_DIR / "manifest.json"
    with open(manifest_path, "w", encoding="utf-8") as f:
        json.dump(manifest, f, indent=2)
        
    print(f"\nSuccessfully generated and frozen {CORPUS_VERSION}!")
    print(f"Total test cases: {len(TEST_CASES)}")
    print(f"Total reference words: {total_words} words")
    print(f"Total audio duration: {total_duration / 60.0:.2f} minutes")
    print(f"Manifest written to: {manifest_path}")

if __name__ == "__main__":
    asyncio.run(generate_all())
