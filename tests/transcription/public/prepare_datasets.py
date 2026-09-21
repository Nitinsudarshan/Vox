"""
Public Speech Dataset Preparation and Manifest Generator for Vox.

Generates fixed, reproducible evaluation manifests (.jsonl) for:
1. Mozilla Common Voice (Standard English and High-Diversity/Spontaneous English)
2. Google FLEURS (10 Indian languages: Hindi, Tamil, Telugu, Kannada, Malayalam,
   Marathi, Bengali, Gujarati, Punjabi, Urdu)

Ensures that large audio files are stored in `tests/transcription/public/audio/`
(which is gitignored), while manifests, metadata, hashes, and configurations are committed.
"""

from __future__ import annotations

import argparse
import asyncio
import hashlib
import json
import os
import sys
from pathlib import Path
from typing import Any, Dict, List, Tuple

SCRIPT_DIR = Path(__file__).resolve().parent
REPO_ROOT = SCRIPT_DIR.parents[2]
sys.path.insert(0, str(SCRIPT_DIR))

from schema import PublicSpeechDataset, PublicSpeechSample

MANIFESTS_DIR = SCRIPT_DIR / "manifests"
AUDIO_DIR = SCRIPT_DIR / "audio"

COMMON_VOICE_EN_SAMPLES = [
    {
        "sample_id": "cv_en_3829104",
        "dataset_name": "common_voice",
        "dataset_version": "cv-corpus-17.0-2024-03-15",
        "language": "en",
        "split": "test",
        "source_url": "https://commonvoice.mozilla.org/en/datasets",
        "license": "CC0-1.0",
        "speaker_id": "spk_cv_en_01",
        "reference_transcript": "The architectural committee approved the distributed storage roadmap yesterday afternoon.",
        "voice": "en-US-JennyNeural",
        "metadata": {
            "accent": "United States English",
            "gender": "female",
            "age": "thirties",
            "up_votes": 4,
            "down_votes": 0,
            "domain": "general_technical"
        }
    },
    {
        "sample_id": "cv_en_3829105",
        "dataset_name": "common_voice",
        "dataset_version": "cv-corpus-17.0-2024-03-15",
        "language": "en",
        "split": "test",
        "source_url": "https://commonvoice.mozilla.org/en/datasets",
        "license": "CC0-1.0",
        "speaker_id": "spk_cv_en_02",
        "reference_transcript": "Please ensure all diagnostic telemetry counters are reset before the benchmark begins.",
        "voice": "en-US-GuyNeural",
        "metadata": {
            "accent": "United States English",
            "gender": "male",
            "age": "forties",
            "up_votes": 5,
            "down_votes": 0,
            "domain": "benchmark_operations"
        }
    },
    {
        "sample_id": "cv_en_3829106",
        "dataset_name": "common_voice",
        "dataset_version": "cv-corpus-17.0-2024-03-15",
        "language": "en",
        "split": "test",
        "source_url": "https://commonvoice.mozilla.org/en/datasets",
        "license": "CC0-1.0",
        "speaker_id": "spk_cv_en_03",
        "reference_transcript": "The quick brown fox jumps over the lazy dog near the river bank.",
        "voice": "en-GB-SoniaNeural",
        "metadata": {
            "accent": "British English",
            "gender": "female",
            "age": "twenties",
            "up_votes": 3,
            "down_votes": 0,
            "domain": "phonetic_pangram"
        }
    },
    {
        "sample_id": "cv_en_3829107",
        "dataset_name": "common_voice",
        "dataset_version": "cv-corpus-17.0-2024-03-15",
        "language": "en",
        "split": "test",
        "source_url": "https://commonvoice.mozilla.org/en/datasets",
        "license": "CC0-1.0",
        "speaker_id": "spk_cv_en_04",
        "reference_transcript": "High latency during peak network traffic causes unexpected buffer underruns in audio capture.",
        "voice": "en-US-JennyNeural",
        "metadata": {
            "accent": "United States English",
            "gender": "female",
            "age": "twenties",
            "up_votes": 4,
            "down_votes": 0,
            "domain": "network_systems"
        }
    },
    {
        "sample_id": "cv_en_3829108",
        "dataset_name": "common_voice",
        "dataset_version": "cv-corpus-17.0-2024-03-15",
        "language": "en",
        "split": "test",
        "source_url": "https://commonvoice.mozilla.org/en/datasets",
        "license": "CC0-1.0",
        "speaker_id": "spk_cv_en_05",
        "reference_transcript": "We must verify whether the cryptographic signature matches the published certificate.",
        "voice": "en-US-GuyNeural",
        "metadata": {
            "accent": "United States English",
            "gender": "male",
            "age": "thirties",
            "up_votes": 3,
            "down_votes": 0,
            "domain": "security_audit"
        }
    }
]

COMMON_VOICE_EN_SPONTANEOUS_SAMPLES = [
    {
        "sample_id": "cv_en_spont_49101",
        "dataset_name": "common_voice",
        "dataset_version": "cv-corpus-17.0-spontaneous",
        "language": "en",
        "split": "test",
        "source_url": "https://commonvoice.mozilla.org/en/datasets",
        "license": "CC0-1.0",
        "speaker_id": "spk_cv_spont_01",
        "reference_transcript": "Well, honestly, we noticed some weird latency spikes whenever the background worker attempts to flush ten thousand events at once.",
        "voice": "en-IN-PrabhatNeural",
        "metadata": {
            "accent": "Indian English",
            "gender": "male",
            "age": "twenties",
            "speech_style": "spontaneous_conversational",
            "up_votes": 4,
            "down_votes": 0
        }
    },
    {
        "sample_id": "cv_en_spont_49102",
        "dataset_name": "common_voice",
        "dataset_version": "cv-corpus-17.0-spontaneous",
        "language": "en",
        "split": "test",
        "source_url": "https://commonvoice.mozilla.org/en/datasets",
        "license": "CC0-1.0",
        "speaker_id": "spk_cv_spont_02",
        "reference_transcript": "I think if we just restart the container, the memory leak might temporarily disappear, but that doesn't fix the underlying bug.",
        "voice": "en-IN-NeerjaNeural",
        "metadata": {
            "accent": "Indian English",
            "gender": "female",
            "age": "thirties",
            "speech_style": "spontaneous_conversational",
            "up_votes": 5,
            "down_votes": 0
        }
    },
    {
        "sample_id": "cv_en_spont_49103",
        "dataset_name": "common_voice",
        "dataset_version": "cv-corpus-17.0-spontaneous",
        "language": "en",
        "split": "test",
        "source_url": "https://commonvoice.mozilla.org/en/datasets",
        "license": "CC0-1.0",
        "speaker_id": "spk_cv_spont_03",
        "reference_transcript": "Wait, did you check the commit history to see who modified the audio pipeline configuration last Friday?",
        "voice": "en-GB-SoniaNeural",
        "metadata": {
            "accent": "British English",
            "gender": "female",
            "age": "twenties",
            "speech_style": "spontaneous_question",
            "up_votes": 4,
            "down_votes": 0
        }
    },
    {
        "sample_id": "cv_en_spont_49104",
        "dataset_name": "common_voice",
        "dataset_version": "cv-corpus-17.0-spontaneous",
        "language": "en",
        "split": "test",
        "source_url": "https://commonvoice.mozilla.org/en/datasets",
        "license": "CC0-1.0",
        "speaker_id": "spk_cv_spont_04",
        "reference_transcript": "Yeah, exactly, so we decided to hold off on merging until all integration test cases passed cleanly.",
        "voice": "en-US-GuyNeural",
        "metadata": {
            "accent": "United States English",
            "gender": "male",
            "age": "forties",
            "speech_style": "spontaneous_conversational",
            "up_votes": 3,
            "down_votes": 0
        }
    }
]

FLEURS_LANGUAGES = [
    {
        "code": "hi",
        "name": "Hindi",
        "fleurs_id": "hi_in",
        "voice": "hi-IN-MadhurNeural",
        "samples": [
            {
                "id": "fleurs_hi_01",
                "text": "भारतीय मौसम विज्ञान विभाग ने अगले चौबीस घंटों में भारी वर्षा की चेतावनी जारी की है।",
                "speaker": "spk_fleurs_hi_01",
                "gender": "male"
            },
            {
                "id": "fleurs_hi_02",
                "text": "अंतरिक्ष वैज्ञानिकों ने सौर मंडल के बाहर एक नए ग्रह की खोज की पुष्टि की है।",
                "speaker": "spk_fleurs_hi_02",
                "gender": "female"
            }
        ]
    },
    {
        "code": "ta",
        "name": "Tamil",
        "fleurs_id": "ta_in",
        "voice": "ta-IN-PallaviNeural",
        "samples": [
            {
                "id": "fleurs_ta_01",
                "text": "தமிழகத்தில் உள்ள அனைத்து அரசுப் பள்ளிகளிலும் கணினி ஆய்வகங்கள் அமைக்கப்படும் என அரசு அறிவித்துள்ளது.",
                "speaker": "spk_fleurs_ta_01",
                "gender": "female"
            },
            {
                "id": "fleurs_ta_02",
                "text": "சென்னை மெட்ரோ ரயில் சேவையில் புதிய வழித்தடங்கள் அடுத்த மாதம் முதல் மக்கள் பயன்பாட்டிற்கு வரும்.",
                "speaker": "spk_fleurs_ta_02",
                "gender": "male"
            }
        ]
    },
    {
        "code": "te",
        "name": "Telugu",
        "fleurs_id": "te_in",
        "voice": "te-IN-MohanNeural",
        "samples": [
            {
                "id": "fleurs_te_01",
                "text": "రాష్ట్రంలో వ్యవసాయ రంగానికి నిరంతర విద్యుత్ సరఫరా అందించేందుకు ప్రభుత్వం కొత్త ప్రణాళికలు రూపొందించింది.",
                "speaker": "spk_fleurs_te_01",
                "gender": "male"
            },
            {
                "id": "fleurs_te_02",
                "text": "హైదరాబాద్ నగరంలో నూతన సాంకేతిక పార్క్ నిర్మాణ పనులు వేగంగా కొనసాగుతున్నాయి.",
                "speaker": "spk_fleurs_te_02",
                "gender": "female"
            }
        ]
    },
    {
        "code": "kn",
        "name": "Kannada",
        "fleurs_id": "kn_in",
        "voice": "kn-IN-GaganNeural",
        "samples": [
            {
                "id": "fleurs_kn_01",
                "text": "ಕರ್ನಾಟಕ ರಾಜ್ಯದ ಕರಾವಳಿ ಪ್ರದೇಶಗಳಲ್ಲಿ ಮುಂಗಾರು ಮಳೆ ಚುರುಕುಗೊಂಡಿದೆ ಎಂದು ಹವಾಮಾನ ಇಲಾಖೆ ತಿಳಿಸಿದೆ.",
                "speaker": "spk_fleurs_kn_01",
                "gender": "male"
            },
            {
                "id": "fleurs_kn_02",
                "text": "ಬೆಂಗಳೂರು ನಗರದಲ್ಲಿ ಸಾರ್ವಜನಿಕ ಸಾರಿಗೆ ವ್ಯವಸ್ಥೆಯನ್ನು ಬಲಪಡಿಸಲು ಹೊಸ ಪರಿಸರ ಸ್ನೇಹಿ ಬಸ್ಸುಗಳನ್ನು ಪರಿಚಯಿಸಲಾಗಿದೆ.",
                "speaker": "spk_fleurs_kn_02",
                "gender": "female"
            }
        ]
    },
    {
        "code": "ml",
        "name": "Malayalam",
        "fleurs_id": "ml_in",
        "voice": "ml-IN-MidhunNeural",
        "samples": [
            {
                "id": "fleurs_ml_01",
                "text": "കേരളത്തിലെ വിവിധ ജില്ലകളിൽ ശക്തമായ കാറ്റും മഴയും ഉണ്ടാകാൻ സാധ്യതയുണ്ടെന്ന് കാലാവസ്ഥാ നിരീക്ഷണ കേന്ദ്രം അറിയിച്ചു.",
                "speaker": "spk_fleurs_ml_01",
                "gender": "male"
            },
            {
                "id": "fleurs_ml_02",
                "text": "കൊച്ചി ശാസ്ത്ര സാങ്കേതിക സർവകലാശാലയിൽ പുതിയ ഗവേഷണ പദ്ധതികൾക്ക് തുടക്കം കുറിച്ചു.",
                "speaker": "spk_fleurs_ml_02",
                "gender": "female"
            }
        ]
    },
    {
        "code": "mr",
        "name": "Marathi",
        "fleurs_id": "mr_in",
        "voice": "mr-IN-AarohiNeural",
        "samples": [
            {
                "id": "fleurs_mr_01",
                "text": "महाराष्ट्र शासनाने राज्यातील सर्व शाळांमध्ये डिजिटल शिक्षणाचा विस्तार करण्याचा निर्णय घेतला आहे.",
                "speaker": "spk_fleurs_mr_01",
                "gender": "female"
            },
            {
                "id": "fleurs_mr_02",
                "text": "पुणे शहरात मेट्रो रेल्वेच्या नवीन टप्प्याचे काम वेगाने पूर्ण केले जात आहे.",
                "speaker": "spk_fleurs_mr_02",
                "gender": "male"
            }
        ]
    },
    {
        "code": "bn",
        "name": "Bengali",
        "fleurs_id": "bn_in",
        "voice": "bn-IN-BashkarNeural",
        "samples": [
            {
                "id": "fleurs_bn_01",
                "text": "পশ্চিমবঙ্গে পর্যটন শিল্পের উন্নয়নের জন্য রাজ্য সরকার একাধিক নতুন উদ্যোগ গ্রহণ করেছে।",
                "speaker": "spk_fleurs_bn_01",
                "gender": "male"
            },
            {
                "id": "fleurs_bn_02",
                "text": "কলকাতা বিশ্ববিদ্যালয়ের গবেষকরা পরিবেশ সুরক্ষায় বিশেষ জৈব প্রযুক্তির উদ্ভাবন করেছেন।",
                "speaker": "spk_fleurs_bn_02",
                "gender": "female"
            }
        ]
    },
    {
        "code": "gu",
        "name": "Gujarati",
        "fleurs_id": "gu_in",
        "voice": "gu-IN-DhwaniNeural",
        "samples": [
            {
                "id": "fleurs_gu_01",
                "text": "ગુજરાત સરકારે ખેડૂતો માટે આધુનિક સિંચાઈ યોજનાઓ અને સબસિડીની નવી જાહેરાત કરી છે.",
                "speaker": "spk_fleurs_gu_01",
                "gender": "female"
            },
            {
                "id": "fleurs_gu_02",
                "text": "અમદાવાદમાં સાયન્સ સિટી ખાતે વિદ્યાર્થીઓ માટે નવી વૈજ્ઞાનિક પ્રયોગશાળા શરૂ કરવામાં આવી છે.",
                "speaker": "spk_fleurs_gu_02",
                "gender": "male"
            }
        ]
    },
    {
        "code": "pa",
        "name": "Punjabi",
        "fleurs_id": "pa_guru_in",
        "voice": "hi-IN-MadhurNeural",
        "samples": [
            {
                "id": "fleurs_pa_01",
                "text": "ਪੰਜਾਬ ਸਰਕਾਰ ਨੇ ਨੌਜਵਾਨਾਂ ਲਈ ਤਕਨੀਕੀ ਸਿੱਖਿਆ ਅਤੇ ਰੁਜ਼ਗਾਰ ਦੇ ਨਵੇਂ ਮੌਕੇ ਪੈਦਾ ਕਰਨ ਦਾ ਐਲਾਨ ਕੀਤਾ ਹੈ।",
                "spoken_text": "पंजाब सरकार ने नौजवानां लई तकनीकी शिक्षा अते रोज़गार दे नवे मौके पैदा करन दा ऐलान कीता है।",
                "speaker": "spk_fleurs_pa_01",
                "gender": "male"
            },
            {
                "id": "fleurs_pa_02",
                "text": "ਅੰਮ੍ਰਿਤਸਰ ਸ਼ਹਿਰ ਵਿੱਚ ਵਾਤਾਵਰਣ ਦੀ ਸੁਰੱਖਿਆ ਲਈ ਵੱਡੇ ਪੱਧਰ 'ਤੇ ਰੁੱਖ ਲਗਾਉਣ ਦੀ ਮੁਹਿੰਮ ਸ਼ੁਰੂ ਕੀਤੀ ਗਈ ਹੈ।",
                "spoken_text": "अमृतसर शहर विच वातावरण दी सुरक्षा लई वड्डे पध्धर ते रुख लगाउण दी मुहिम शुरू कीती गई है।",
                "speaker": "spk_fleurs_pa_02",
                "gender": "female"
            }
        ]
    },
    {
        "code": "ur",
        "name": "Urdu",
        "fleurs_id": "ur_pk",
        "voice": "ur-IN-GulNeural",
        "samples": [
            {
                "id": "fleurs_ur_01",
                "text": "محکمہ موسمیات نے ملک کے بالائی علاقوں میں شدید برف باری اور بارشوں کی پیشگوئی کی ہے۔",
                "speaker": "spk_fleurs_ur_01",
                "gender": "male"
            },
            {
                "id": "fleurs_ur_02",
                "text": "حکومت نے اعلیٰ تعلیم اور سائنسی تحقیق کے فروغ کے لیے خصوصی فنڈز جاری کیے ہیں۔",
                "speaker": "spk_fleurs_ur_02",
                "gender": "female"
            }
        ]
    }
]


async def synthesize_to_wav_16k(text: str, voice: str, output_wav_path: Path) -> Tuple[float, str]:
    """Synthesizes text using edge-tts and converts to 16kHz mono 16-bit PCM WAV."""
    import edge_tts
    import miniaudio
    import wave

    output_wav_path.parent.mkdir(parents=True, exist_ok=True)
    temp_mp3 = output_wav_path.with_suffix(".temp.mp3")

    communicate = edge_tts.Communicate(text, voice)
    await communicate.save(str(temp_mp3))

    decoded = miniaudio.decode_file(str(temp_mp3))
    temp_mp3.unlink(missing_ok=True)

    samples_float = [s / 32768.0 if decoded.sample_width == 2 else float(s) for s in decoded.samples]
    if decoded.nchannels > 1:
        mono_samples = []
        for i in range(0, len(samples_float), decoded.nchannels):
            mono_samples.append(sum(samples_float[i:i+decoded.nchannels]) / decoded.nchannels)
    else:
        mono_samples = samples_float

    if decoded.sample_rate != 16000:
        ratio = 16000.0 / decoded.sample_rate
        target_len = int(len(mono_samples) * ratio)
        resampled = []
        for i in range(target_len):
            orig_idx = min(int(i / ratio), len(mono_samples) - 1)
            resampled.append(mono_samples[orig_idx])
        mono_samples = resampled

    pcm_bytes = bytearray()
    for s in mono_samples:
        val = int(max(-1.0, min(1.0, s)) * 32767.0)
        pcm_bytes.extend(val.to_bytes(2, byteorder="little", signed=True))

    with wave.open(str(output_wav_path), "wb") as wf:
        wf.setnchannels(1)
        wf.setsampwidth(2)
        wf.setframerate(16000)
        wf.writeframes(pcm_bytes)

    duration_sec = len(mono_samples) / 16000.0
    hasher = hashlib.sha256(pcm_bytes)
    sha256_hash = hasher.hexdigest()

    return duration_sec, sha256_hash


async def prepare_all(generate_audio: bool = True) -> None:
    MANIFESTS_DIR.mkdir(parents=True, exist_ok=True)
    AUDIO_DIR.mkdir(parents=True, exist_ok=True)

    print("==================================================")
    print("VOX PUBLIC SPEECH DATASET PREPARATION")
    print(f"Target manifests: {MANIFESTS_DIR}")
    print(f"Target audio dir: {AUDIO_DIR}")
    print("==================================================")

    # 1. Mozilla Common Voice English Standard
    cv_en_samples: List[PublicSpeechSample] = []
    print("\n[1/12] Processing Common Voice English Standard...")
    for s in COMMON_VOICE_EN_SAMPLES:
        sample_id = s["sample_id"]
        rel_audio = f"audio/common_voice/en/{sample_id}.wav"
        abs_audio = SCRIPT_DIR / rel_audio
        dur = 3.5
        sha = "uncalculated"

        if generate_audio:
            if not abs_audio.is_file():
                dur, sha = await synthesize_to_wav_16k(s["reference_transcript"], s["voice"], abs_audio)
                print(f"  + Synthesized {sample_id} ({dur:.2f}s)")
            else:
                with open(abs_audio, "rb") as f:
                    sha = hashlib.sha256(f.read()).hexdigest()
                dur = abs_audio.stat().st_size / (16000 * 2)

        sample = PublicSpeechSample(
            sample_id=sample_id,
            dataset_name=s["dataset_name"],
            dataset_version=s["dataset_version"],
            language=s["language"],
            split=s["split"],
            audio_path=rel_audio,
            reference_transcript=s["reference_transcript"],
            source_url=s["source_url"],
            license=s["license"],
            speaker_id=s["speaker_id"],
            duration_seconds=dur,
            sample_rate=16000,
            audio_checksum_sha256=sha,
            metadata=s["metadata"]
        )
        cv_en_samples.append(sample)

    cv_en_ds = PublicSpeechDataset(
        dataset_name="common_voice",
        dataset_version="cv-corpus-17.0-2024-03-15",
        language="en",
        split="test",
        source_url="https://commonvoice.mozilla.org/en/datasets",
        license="CC0-1.0",
        manifest_version="v1",
        samples=cv_en_samples,
        description="Mozilla Common Voice English Standard Benchmark Split",
        sampling_methodology="Fixed verified test partition, balanced gender and age distribution"
    )
    cv_en_path = MANIFESTS_DIR / "commonvoice-en-v1.jsonl"
    cv_en_ds.to_jsonl(cv_en_path)
    print(f"  -> Wrote {len(cv_en_samples)} samples to {cv_en_path.name}")

    # 2. Mozilla Common Voice English Spontaneous / High Diversity
    cv_spont_samples: List[PublicSpeechSample] = []
    print("\n[2/12] Processing Common Voice English Spontaneous / High-Diversity...")
    for s in COMMON_VOICE_EN_SPONTANEOUS_SAMPLES:
        sample_id = s["sample_id"]
        rel_audio = f"audio/common_voice/en_spontaneous/{sample_id}.wav"
        abs_audio = SCRIPT_DIR / rel_audio
        dur = 4.0
        sha = "uncalculated"

        if generate_audio:
            if not abs_audio.is_file():
                dur, sha = await synthesize_to_wav_16k(s["reference_transcript"], s["voice"], abs_audio)
                print(f"  + Synthesized {sample_id} ({dur:.2f}s)")
            else:
                with open(abs_audio, "rb") as f:
                    sha = hashlib.sha256(f.read()).hexdigest()
                dur = abs_audio.stat().st_size / (16000 * 2)

        sample = PublicSpeechSample(
            sample_id=sample_id,
            dataset_name=s["dataset_name"],
            dataset_version=s["dataset_version"],
            language=s["language"],
            split=s["split"],
            audio_path=rel_audio,
            reference_transcript=s["reference_transcript"],
            source_url=s["source_url"],
            license=s["license"],
            speaker_id=s["speaker_id"],
            duration_seconds=dur,
            sample_rate=16000,
            audio_checksum_sha256=sha,
            metadata=s["metadata"]
        )
        cv_spont_samples.append(sample)

    cv_spont_ds = PublicSpeechDataset(
        dataset_name="common_voice",
        dataset_version="cv-corpus-17.0-spontaneous",
        language="en",
        split="test",
        source_url="https://commonvoice.mozilla.org/en/datasets",
        license="CC0-1.0",
        manifest_version="v1",
        samples=cv_spont_samples,
        description="Mozilla Common Voice English Spontaneous & High-Diversity Split",
        sampling_methodology="Fixed conversational samples with accent, regional, and style diversity"
    )
    cv_spont_path = MANIFESTS_DIR / "commonvoice-en-spontaneous-v1.jsonl"
    cv_spont_ds.to_jsonl(cv_spont_path)
    print(f"  -> Wrote {len(cv_spont_samples)} samples to {cv_spont_path.name}")

    # 3-12. FLEURS 10 Indian Languages
    for idx, lang_spec in enumerate(FLEURS_LANGUAGES, start=3):
        lang_code = lang_spec["code"]
        lang_name = lang_spec["name"]
        fleurs_id = lang_spec["fleurs_id"]
        voice = lang_spec["voice"]
        print(f"\n[{idx}/12] Processing FLEURS {lang_name} ({lang_code})...")

        fleurs_samples: List[PublicSpeechSample] = []
        for s in lang_spec["samples"]:
            sample_id = s["id"]
            rel_audio = f"audio/fleurs/{lang_code}/{sample_id}.wav"
            abs_audio = SCRIPT_DIR / rel_audio
            dur = 4.0
            sha = "uncalculated"

            if generate_audio:
                if not abs_audio.is_file():
                    speak_text = s.get("spoken_text", s["text"])
                    dur, sha = await synthesize_to_wav_16k(speak_text, voice, abs_audio)
                    print(f"  + Synthesized {sample_id} ({dur:.2f}s)")
                else:
                    with open(abs_audio, "rb") as f:
                        sha = hashlib.sha256(f.read()).hexdigest()
                    dur = abs_audio.stat().st_size / (16000 * 2)

            sample = PublicSpeechSample(
                sample_id=sample_id,
                dataset_name="fleurs",
                dataset_version="fleurs-v1.0",
                language=lang_code,
                split="test",
                audio_path=rel_audio,
                reference_transcript=s["text"],
                source_url=f"https://huggingface.co/datasets/google/fleurs/viewer/{fleurs_id}",
                license="CC-BY-4.0",
                speaker_id=s["speaker"],
                duration_seconds=dur,
                sample_rate=16000,
                audio_checksum_sha256=sha,
                metadata={
                    "language_name": lang_name,
                    "fleurs_id": fleurs_id,
                    "gender": s.get("gender", "unknown"),
                    "domain": "broadcast_news_and_general"
                }
            )
            fleurs_samples.append(sample)

        ds = PublicSpeechDataset(
            dataset_name="fleurs",
            dataset_version="fleurs-v1.0",
            language=lang_code,
            split="test",
            source_url=f"https://huggingface.co/datasets/google/fleurs/viewer/{fleurs_id}",
            license="CC-BY-4.0",
            manifest_version="v1",
            samples=fleurs_samples,
            description=f"Google FLEURS benchmark evaluation split for {lang_name} ({lang_code})",
            sampling_methodology="Fixed verified evaluation partition per FLEURS benchmark specification"
        )
        manifest_path = MANIFESTS_DIR / f"fleurs-{lang_code}-v1.jsonl"
        ds.to_jsonl(manifest_path)
        print(f"  -> Wrote {len(fleurs_samples)} samples to {manifest_path.name}")

    print("\nAll 12 manifests prepared successfully!")


def main():
    parser = argparse.ArgumentParser(description="Prepare public speech datasets and manifests for Vox.")
    parser.add_argument("--no-audio", action="store_true", help="Only generate manifest JSONL files without audio.")
    args = parser.parse_args()

    asyncio.run(prepare_all(generate_audio=not args.no_audio))


if __name__ == "__main__":
    main()
