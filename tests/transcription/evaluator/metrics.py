"""
Evaluation Metrics for Vox Long-Form Meeting Transcription Benchmark.
Computes WER, CER, Insertions, Deletions, Substitutions, Hallucination Rate,
Code-Switch Accuracy, and automated failure classification.
"""

import re
import unicodedata
from typing import Any, Dict, List, Tuple

# Domain keywords monitored for specialized domain vocabulary accuracy
DOMAIN_KEYWORDS = [
    "navgurukul", "sosc", "ngconnect", "ghar", "sama", "saathi",
    "ares", "macquarie", "kishore", "abhishek", "tauri", "rust",
    "whisper", "supabase", "whisper.cpp", "cpal"
]

def normalize_text(text: str) -> str:
    """Normalizes text for robust WER calculation: lowercase, NFKC unicode, strip punctuation."""
    if not text:
        return ""
    text = unicodedata.normalize("NFKC", text).lower()
    # Replace hyphens and slashes with space
    text = re.sub(r"[-_/\\]", " ", text)
    # Remove standard punctuation while keeping Devanagari and Latin characters
    text = re.sub(r"[^\w\s\u0900-\u097F]", "", text)
    # Collapse multiple spaces
    return re.sub(r"\s+", " ", text).strip()

def compute_levenshtein(ref_tokens: List[str], hyp_tokens: List[str]) -> Tuple[int, int, int, int]:
    """
    Computes Levenshtein alignment between reference and hypothesis tokens.
    Returns: (substitutions, deletions, insertions, total_errors)
    """
    n, m = len(ref_tokens), len(hyp_tokens)
    dp = [[0] * (m + 1) for _ in range(n + 1)]
    ops = [[None] * (m + 1) for _ in range(n + 1)]

    for i in range(n + 1):
        dp[i][0] = i
        ops[i][0] = 'D'
    for j in range(m + 1):
        dp[0][j] = j
        ops[0][j] = 'I'
    ops[0][0] = ' '

    for i in range(1, n + 1):
        for j in range(1, m + 1):
            if ref_tokens[i - 1] == hyp_tokens[j - 1]:
                dp[i][j] = dp[i - 1][j - 1]
                ops[i][j] = 'M'  # Match
            else:
                sub = dp[i - 1][j - 1] + 1
                delete = dp[i - 1][j] + 1
                insert = dp[i][j - 1] + 1
                min_cost = min(sub, delete, insert)
                dp[i][j] = min_cost
                if min_cost == sub:
                    ops[i][j] = 'S'
                elif min_cost == delete:
                    ops[i][j] = 'D'
                else:
                    ops[i][j] = 'I'

    # Backtrace to count S, D, I
    subs, dels, inss = 0, 0, 0
    i, j = n, m
    while i > 0 or j > 0:
        op = ops[i][j]
        if op == 'M':
            i -= 1
            j -= 1
        elif op == 'S':
            subs += 1
            i -= 1
            j -= 1
        elif op == 'D':
            dels += 1
            i -= 1
        elif op == 'I':
            inss += 1
            j -= 1
        else:
            break

    total_errors = subs + dels + inss
    return subs, dels, inss, total_errors

def calculate_wer(reference: str, hypothesis: str) -> Dict[str, Any]:
    """Calculates Word Error Rate (WER) and token error breakdown."""
    norm_ref = normalize_text(reference)
    norm_hyp = normalize_text(hypothesis)

    ref_words = norm_ref.split() if norm_ref else []
    hyp_words = norm_hyp.split() if norm_hyp else []

    if not ref_words:
        return {
            "wer": 0.0 if not hyp_words else 1.0,
            "ref_words": 0,
            "hyp_words": len(hyp_words),
            "substitutions": 0,
            "deletions": 0,
            "insertions": len(hyp_words),
            "total_errors": len(hyp_words)
        }

    subs, dels, inss, total_errors = compute_levenshtein(ref_words, hyp_words)
    wer = total_errors / float(len(ref_words))

    return {
        "wer": round(wer, 4),
        "wer_percent": round(wer * 100.0, 2),
        "ref_words": len(ref_words),
        "hyp_words": len(hyp_words),
        "substitutions": subs,
        "deletions": dels,
        "insertions": inss,
        "total_errors": total_errors
    }

def calculate_cer(reference: str, hypothesis: str) -> Dict[str, Any]:
    """Calculates Character Error Rate (CER) and character error breakdown."""
    norm_ref = normalize_text(reference).replace(" ", "")
    norm_hyp = normalize_text(hypothesis).replace(" ", "")

    ref_chars = list(norm_ref)
    hyp_chars = list(norm_hyp)

    if not ref_chars:
        return {
            "cer": 0.0 if not hyp_chars else 1.0,
            "ref_chars": 0,
            "hyp_chars": len(hyp_chars),
            "substitutions": 0,
            "deletions": 0,
            "insertions": len(hyp_chars),
            "total_errors": len(hyp_chars)
        }

    subs, dels, inss, total_errors = compute_levenshtein(ref_chars, hyp_chars)
    cer = total_errors / float(len(ref_chars))

    return {
        "cer": round(cer, 4),
        "cer_percent": round(cer * 100.0, 2),
        "ref_chars": len(ref_chars),
        "hyp_chars": len(hyp_chars),
        "substitutions": subs,
        "deletions": dels,
        "insertions": inss,
        "total_errors": total_errors
    }

def is_devanagari_word(word: str) -> bool:
    """Checks if a word is composed primarily of Devanagari Unicode characters."""
    return any('\u0900' <= char <= '\u097F' for char in word)

def calculate_codeswitch_accuracy(reference: str, hypothesis: str) -> Dict[str, Any]:
    """
    Evaluates Hinglish code-switching retention and language breakdown:
    - English word retention vs Hindi (Devanagari or Romanized) word retention.
    - Technical domain term recognition.
    """
    ref_norm = normalize_text(reference)
    hyp_norm = normalize_text(hypothesis)

    ref_words = ref_norm.split()
    hyp_words = set(hyp_norm.split())

    # Detect Devanagari vs Latin words in reference
    hindi_words = [w for w in ref_words if is_devanagari_word(w)]
    english_words = [w for w in ref_words if not is_devanagari_word(w)]

    matched_hindi = sum(1 for w in hindi_words if w in hyp_words)
    matched_english = sum(1 for w in english_words if w in hyp_words)

    hindi_retention = (matched_hindi / len(hindi_words)) if hindi_words else 1.0
    english_retention = (matched_english / len(english_words)) if english_words else 1.0

    # Technical / domain terms check
    found_domain = {}
    for term in DOMAIN_KEYWORDS:
        if term in ref_norm:
            found_domain[term] = (term in hyp_norm)

    domain_accuracy = (
        sum(1 for v in found_domain.values() if v) / len(found_domain)
        if found_domain else 1.0
    )

    return {
        "hindi_words_count": len(hindi_words),
        "hindi_retention_percent": round(hindi_retention * 100.0, 2),
        "english_words_count": len(english_words),
        "english_retention_percent": round(english_retention * 100.0, 2),
        "domain_terms_monitored": found_domain,
        "domain_accuracy_percent": round(domain_accuracy * 100.0, 2)
    }

def calculate_hallucination_rate(
    reference: str, hypothesis: str, telemetry_list: List[Dict[str, Any]] = None
) -> Dict[str, Any]:
    """
    Measures hallucination rate based on unexpected token insertions
    and segments flagged with high compression ratio or repetitive phrases.
    """
    wer_data = calculate_wer(reference, hypothesis)
    hyp_words = wer_data["hyp_words"]
    insertions = wer_data["insertions"]

    # Insertion-based hallucination proxy
    insertion_hallucination_rate = (insertions / float(hyp_words)) if hyp_words > 0 else 0.0

    suspicious_segments = 0
    rejected_segments = 0
    total_segments = len(telemetry_list) if telemetry_list else 0

    if telemetry_list:
        for t in telemetry_list:
            status = str(t.get("quality_status", "")).lower()
            if "suspicious" in status or "recovered" in status:
                suspicious_segments += 1
            elif "rejected" in status or "discarded" in status:
                rejected_segments += 1

    return {
        "insertion_hallucination_rate_percent": round(insertion_hallucination_rate * 100.0, 2),
        "suspicious_segments_count": suspicious_segments,
        "rejected_segments_count": rejected_segments,
        "total_segments_evaluated": total_segments
    }

def classify_transcription_failure(
    case_meta: Dict[str, Any],
    wer_data: Dict[str, Any],
    cer_data: Dict[str, Any],
    telemetry_list: List[Dict[str, Any]] = None
) -> Dict[str, Any]:
    """
    Classifies failure causes into systematic categories:
    AUDIO, VAD, SEGMENTATION, LANGUAGE, DECODING, HALLUCINATION, VOCABULARY, CONTEXT, MODEL, UNKNOWN.
    """
    reasons = []
    primary_category = "NONE"

    wer = wer_data.get("wer", 0.0)
    inss = wer_data.get("insertions", 0)
    dels = wer_data.get("deletions", 0)
    subs = wer_data.get("substitutions", 0)
    hyp_words = wer_data.get("hyp_words", 0)
    ref_words = wer_data.get("ref_words", 0)

    # 1. Hallucination check
    if inss > 0.25 * ref_words or (hyp_words > 1.35 * ref_words and inss > subs):
        primary_category = "HALLUCINATION"
        reasons.append("Excessive insertions detected indicating hallucination loops or phantom tokens.")

    # 2. Audio/Noise check
    elif case_meta.get("added_ambient_noise", False) and wer > 0.25:
        primary_category = "AUDIO"
        reasons.append("Background ambient noise corrupted acoustic features or raised noise floor.")

    # 3. VAD / Deletions check
    elif dels > 0.3 * ref_words and hyp_words < 0.7 * ref_words:
        primary_category = "VAD"
        reasons.append("Severe word deletions detected indicating premature VAD silence cutoff or unclosed utterance.")

    # 4. Language mismatch check
    elif case_meta.get("language") == "hinglish" and wer > 0.22:
        primary_category = "LANGUAGE"
        reasons.append("Intra-sentence code-switching caused phonetic substitution across English/Hindi boundaries.")

    # 5. Domain Vocabulary check
    elif case_meta.get("category") == "G_tech_domain" and subs > 0.15 * ref_words:
        primary_category = "VOCABULARY"
        reasons.append("Specialized proper nouns and domain technical terms experienced phonetic substitution.")

    # 6. High substitutions general model capacity
    elif subs > 0.25 * ref_words:
        primary_category = "MODEL"
        reasons.append("High substitution rate indicates acoustic model acoustic-to-text capacity ceiling.")

    elif wer > 0.15:
        primary_category = "DECODING"
        reasons.append("Sub-optimal beam search or temperature fallback during ambiguous speech intervals.")

    return {
        "primary_category": primary_category,
        "reasons": reasons,
        "confidence": "high" if primary_category != "UNKNOWN" else "low"
    }
