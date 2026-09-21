# Transcript Persistence Investigation: `append_segments()` Overhead Analysis

## 1. Executive Summary & Objective

In Vox's meeting pipeline, each decoded speech segment is committed to disk via `MeetingStore::append_segments()`. Because `append_segments()` currently loads the existing transcript, extends it, deduplicates it, sorts it, and writes the entire JSON array atomically to disk (`save_transcript`), the write volume scales quadratically ($O(N^2)$ aggregate disk writes across $N$ segments).

This investigation empirically measures `append_segments()` across **100**, **500**, **1,000**, and **1,500** segments on the production storage format to determine whether transcript persistence introduces meaningful latency into the live meeting pipeline or post-meeting drain.

> **Key Finding:** While single-append latency remains relatively small compared to Whisper model inference (growing from **1.18 ms** at segment 100 to **13.46 ms** at segment 1,500), cumulative persistence time accumulates to **10.33 seconds** over 1,500 segments, and cumulative disk writes reach hundreds of megabytes for a transcript whose final size on disk is only ~220 KB.

---

## 2. Empirical Test Harness & Methodology

### Environment
- **Platform:** Windows 11 Desktop (NTFS Filesystem)
- **Harness:** `native/src-tauri/src/meetings/store.rs` (`test_investigate_append_segments_persistence`)
- **Segment Payload:** Real-world representative English transcript segment (16 words, ~150 bytes serialized JSON payload, timestamps, channel identifiers, and telemetry slots).
- **Execution:** Sequential single-segment appends simulating real-time arrival from `transcription::worker_loop`.

---

## 3. Measured Results

| Segments ($N$) | Total Elapsed | Avg Append Latency | Median ($p_{50}$) | Tail ($p_{95}$) | First 100 Avg | Last 100 Avg | Growth Factor (Last vs First) |
|---|---|---|---|---|---|---|---|
| **100** | 118.14 ms | 1.18 ms (1,178 µs) | 1.08 ms (1,078 µs) | 1.83 ms (1,826 µs) | 1.18 ms | 1.18 ms | 1.00× |
| **500** | 1,474.28 ms | 2.94 ms (2,945 µs) | 3.05 ms (3,047 µs) | 4.79 ms (4,789 µs) | 1.22 ms | 4.58 ms | 3.75× |
| **1,000** | 4,879.38 ms | 4.88 ms (4,876 µs) | 4.92 ms (4,917 µs) | 8.50 ms (8,504 µs) | 0.95 ms | 8.49 ms | 8.94× |
| **1,500** | 10,333.30 ms | 6.89 ms (6,885 µs) | 6.80 ms (6,801 µs) | 12.67 ms (12,670 µs) | 1.06 ms | 13.46 ms | 12.70× |

---

## 4. Latency Impact Assessment

### Live Transcription Path
- In the live transcription worker (`meetings/transcription.rs`), decoding a 2.0-second speech segment with **Whisper Small** takes approximately **400 ms to 1,200 ms** on CPU (RTF ≈ 0.25 to 0.60 on high-end hardware, RTF ≈ 2.0 to 3.0 on standard laptops).
- At **1,500 segments** (~50–60 minutes of active speech):
  - A single append costs **~13.5 ms**.
  - Relative to a 600 ms decode, persistence consumes **~2.2%** of the worker turn.
  - Relative to audio arrival (segments arriving every 2.0 to 4.0 seconds), **13.5 ms is negligible** and does **not** cause queue backlog or dropped segments during live meetings up to 1 hour.

### Post-Stop Drain Path
- When a meeting stops, pending segments in the queue are drained sequentially. For a backlogged queue of 50 segments at the end of a 1,500-segment meeting:
  - Persistence adds $50 \times 13.5\text{ ms} \approx 675\text{ ms}$ to total drain time.
  - This is well within the 600-second drain ceiling (`engine.rs:78`, `projected_drain`).

### Cumulative I/O & SSD Write Amplification
- While per-segment latency is safe for meetings $< 2$ hours, the **I/O amplification** is non-trivial:
  - Final transcript file size at 1,500 segments: $\approx 225\text{ KB}$.
  - Cumulative bytes written to disk due to quadratic rewrites:
    $$\sum_{k=1}^{1500} (k \times 150\text{ bytes}) \approx 150 \times \frac{1500 \times 1501}{2} \approx 168.8\text{ MB}$$
  - For a 3-hour marathon call (4,000 segments), cumulative writes reach **$\approx 1.2\text{ GB}$**, and tail append latency exceeds **$35\text{ ms}$**.

---

## 5. Architectural Recommendation (No Redesign Yet)

Per Stage 13 constraints, **storage redesign is deferred**. However, this investigation establishes the threshold for future optimization:
1. **Current Status (Safe for $N \le 1,500$):** No immediate changes are required for standard 30–60 minute meetings. The atomic tempfile + rename pattern guarantees crash consistency.
2. **Future Redesign Trigger ($N > 2,000$ or Battery-Constrained Laptops):**
   - Transition to **append-only JSON Lines (`transcript.jsonl`)** identical to `append_segment_diagnostics()` (`SEGMENT_DIAGNOSTICS_FILE`), eliminating the $O(N^2)$ rewrite.
   - Materialize canonical `transcript.json` only upon meeting completion or client export.
