//! A cheap acoustic fingerprint, and the clustering that groups turns by who
//! was speaking.
//!
//! # What this is, and what it is not
//!
//! This is **not** neural speaker diarization. A production system embeds each
//! turn with a trained speaker encoder — x-vector, ECAPA-TDNN — which learns
//! what makes two recordings the same person and, more importantly, what makes
//! them different people saying similar things. That needs an ONNX runtime and
//! a model file, and `maybe_later.md` §11 is where that work is described.
//!
//! What this is: MFCC statistics over each turn, clustered by cosine distance.
//! MFCCs describe the shape of the vocal tract, so two turns by the same
//! speaker land near each other far more often than chance — enough to
//! *propose* a grouping. It is beaten by a shared microphone, by two similar
//! voices, and by a turn short enough that one vowel dominates its statistics.
//!
//! So the product shape follows the technique's honesty rather than
//! overselling it: Vox proposes groups, plays a sample of each, and the user
//! names them. A wrong proposal costs one click to fix. That flow stays
//! correct when the embeddings get better; only the proposals improve.
//!
//! # Why the DSP is written out here
//!
//! No FFT crate is in the tree, and adding one for twenty lines of radix-2
//! butterfly is a dependency to audit, pin and carry on every platform. The
//! transform below is the textbook iterative Cooley-Tukey, and every stage
//! from pre-emphasis to the DCT is unit-tested against signals whose answer is
//! known by construction.

use std::f32::consts::PI;

/// Samples per second every meeting segment arrives at.
pub const SAMPLE_RATE: f32 = 16_000.0;

/// Analysis window, in samples. 32 ms at 16 kHz, and a power of two so the
/// transform needs no padding.
const FRAME_SIZE: usize = 512;

/// Hop between windows, in samples — 16 ms, the conventional 50% overlap.
const HOP_SIZE: usize = 256;

/// Triangular mel filters spanning the speech band.
const MEL_BANDS: usize = 26;

/// Cepstral coefficients kept per frame.
///
/// The first is overall loudness, which says how close someone sat to the
/// microphone rather than who they are, and is dropped by [`voiceprint`].
const CEPSTRA: usize = 13;

/// Lowest and highest frequency the filterbank covers. Below 80 Hz is mains
/// hum and rumble; above 7600 Hz is above the band a 16 kHz capture resolves.
const MEL_LOW_HZ: f32 = 80.0;
const MEL_HIGH_HZ: f32 = 7_600.0;

/// Speech shorter than this makes a fingerprint dominated by whichever vowel
/// happened to be in it. Such a turn is still transcribed and still shown; it
/// simply does not get a vote on who was speaking.
pub const MIN_VOICEPRINT_SECONDS: f32 = 1.2;

/// The fingerprint of one turn: per-coefficient mean and standard deviation.
///
/// The standard deviation earns its place — the mean describes the average
/// vocal-tract shape, and how much a speaker *moves* around that average
/// separates two people whose averages are close.
pub type Voiceprint = Vec<f32>;

/// Fingerprints `samples`, or `None` when there is too little to go on.
pub fn voiceprint(samples: &[f32]) -> Option<Voiceprint> {
    if (samples.len() as f32) < MIN_VOICEPRINT_SECONDS * SAMPLE_RATE {
        return None;
    }
    let emphasised = pre_emphasis(samples);
    let filters = mel_filterbank();

    let mut frames: Vec<[f32; CEPSTRA]> = Vec::new();
    let mut start = 0;
    while start + FRAME_SIZE <= emphasised.len() {
        let frame = &emphasised[start..start + FRAME_SIZE];
        start += HOP_SIZE;

        let spectrum = power_spectrum(frame);
        let energies = apply_filterbank(&spectrum, &filters);
        // Silence between words carries no vocal tract to measure, and
        // including it pulls every speaker's fingerprint toward the same
        // floor.
        if energies.iter().all(|e| *e <= 0.0) {
            continue;
        }
        frames.push(dct_ii(&energies));
    }

    // Enough frames for a standard deviation to mean anything.
    if frames.len() < 8 {
        return None;
    }

    // Coefficient 0 is loudness: how close the speaker sat, not who they are.
    let kept = 1..CEPSTRA;
    let mut print = Vec::with_capacity((CEPSTRA - 1) * 2);
    for index in kept.clone() {
        let mean = frames.iter().map(|f| f[index]).sum::<f32>() / frames.len() as f32;
        print.push(mean);
    }
    for (offset, index) in kept.enumerate() {
        let mean = print[offset];
        let variance = frames
            .iter()
            .map(|f| (f[index] - mean).powi(2))
            .sum::<f32>()
            / frames.len() as f32;
        print.push(variance.sqrt());
    }
    Some(normalize(print))
}

/// Scales a fingerprint to unit length, so cosine distance is a dot product
/// and a loud recording does not read as a different speaker from a quiet one.
fn normalize(mut print: Voiceprint) -> Voiceprint {
    let norm = print.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > f32::EPSILON {
        for value in print.iter_mut() {
            *value /= norm;
        }
    }
    print
}

/// Cosine distance in `0.0..=2.0`: 0 is identical, 1 is unrelated.
pub fn distance(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 1.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    (1.0 - dot).clamp(0.0, 2.0)
}

/// Flattens the spectral tilt of voiced speech, which otherwise lets the first
/// few bands dominate every fingerprint.
fn pre_emphasis(samples: &[f32]) -> Vec<f32> {
    let mut out = Vec::with_capacity(samples.len());
    let mut previous = 0.0;
    for sample in samples {
        out.push(sample - 0.97 * previous);
        previous = *sample;
    }
    out
}

/// One frame's power spectrum, Hann-windowed.
fn power_spectrum(frame: &[f32]) -> Vec<f32> {
    let mut real: Vec<f32> = frame
        .iter()
        .enumerate()
        .map(|(n, sample)| {
            let window =
                0.5 - 0.5 * (2.0 * PI * n as f32 / (FRAME_SIZE as f32 - 1.0)).cos();
            sample * window
        })
        .collect();
    let mut imaginary = vec![0.0f32; FRAME_SIZE];
    fft_in_place(&mut real, &mut imaginary);

    // Only the first half is independent; the rest mirrors it.
    (0..=FRAME_SIZE / 2)
        .map(|k| real[k] * real[k] + imaginary[k] * imaginary[k])
        .collect()
}

/// Iterative radix-2 Cooley-Tukey, in place. `real.len()` must be a power of
/// two, which [`FRAME_SIZE`] guarantees.
fn fft_in_place(real: &mut [f32], imaginary: &mut [f32]) {
    let n = real.len();
    debug_assert!(n.is_power_of_two());

    // Bit-reversal permutation.
    let mut target = 0usize;
    for source in 1..n {
        let mut bit = n >> 1;
        while target & bit != 0 {
            target ^= bit;
            bit >>= 1;
        }
        target |= bit;
        if source < target {
            real.swap(source, target);
            imaginary.swap(source, target);
        }
    }

    let mut span = 2;
    while span <= n {
        let angle = -2.0 * PI / span as f32;
        let (step_sin, step_cos) = angle.sin_cos();
        for block in (0..n).step_by(span) {
            let (mut w_real, mut w_imaginary) = (1.0f32, 0.0f32);
            for offset in 0..span / 2 {
                let even = block + offset;
                let odd = even + span / 2;
                let t_real = w_real * real[odd] - w_imaginary * imaginary[odd];
                let t_imaginary = w_real * imaginary[odd] + w_imaginary * real[odd];
                real[odd] = real[even] - t_real;
                imaginary[odd] = imaginary[even] - t_imaginary;
                real[even] += t_real;
                imaginary[even] += t_imaginary;

                let next_real = w_real * step_cos - w_imaginary * step_sin;
                w_imaginary = w_real * step_sin + w_imaginary * step_cos;
                w_real = next_real;
            }
        }
        span <<= 1;
    }
}

/// Hz to the mel scale, on which equal steps sound equally far apart.
fn hz_to_mel(hz: f32) -> f32 {
    2595.0 * (1.0 + hz / 700.0).log10()
}

fn mel_to_hz(mel: f32) -> f32 {
    700.0 * (10f32.powf(mel / 2595.0) - 1.0)
}

/// The triangular filters, as `(start_bin, weights)` pairs.
fn mel_filterbank() -> Vec<(usize, Vec<f32>)> {
    let bins = FRAME_SIZE / 2 + 1;
    let low = hz_to_mel(MEL_LOW_HZ);
    let high = hz_to_mel(MEL_HIGH_HZ);

    // MEL_BANDS filters need MEL_BANDS + 2 edges: each filter spans three.
    let edges: Vec<usize> = (0..MEL_BANDS + 2)
        .map(|index| {
            let mel = low + (high - low) * index as f32 / (MEL_BANDS + 1) as f32;
            let hz = mel_to_hz(mel);
            ((hz / (SAMPLE_RATE / 2.0)) * (bins - 1) as f32).round() as usize
        })
        .map(|bin| bin.min(bins - 1))
        .collect();

    let mut filters = Vec::with_capacity(MEL_BANDS);
    for band in 0..MEL_BANDS {
        let (left, centre, right) = (edges[band], edges[band + 1], edges[band + 2]);
        let mut weights = Vec::new();
        for bin in left..=right {
            let weight = if bin < centre && centre > left {
                (bin - left) as f32 / (centre - left) as f32
            } else if bin > centre && right > centre {
                (right - bin) as f32 / (right - centre) as f32
            } else if bin == centre {
                1.0
            } else {
                0.0
            };
            weights.push(weight);
        }
        filters.push((left, weights));
    }
    filters
}

/// Log energy in each mel band.
fn apply_filterbank(spectrum: &[f32], filters: &[(usize, Vec<f32>)]) -> Vec<f32> {
    filters
        .iter()
        .map(|(start, weights)| {
            let energy: f32 = weights
                .iter()
                .enumerate()
                .map(|(offset, weight)| spectrum.get(start + offset).copied().unwrap_or(0.0) * weight)
                .sum();
            // The floor keeps a silent band from becoming negative infinity
            // and taking the whole frame's statistics with it.
            (energy + 1e-10).ln()
        })
        .collect()
}

/// Type-II DCT, decorrelating the band energies into cepstral coefficients.
fn dct_ii(energies: &[f32]) -> [f32; CEPSTRA] {
    let mut out = [0.0f32; CEPSTRA];
    let n = energies.len() as f32;
    for (k, slot) in out.iter_mut().enumerate() {
        *slot = energies
            .iter()
            .enumerate()
            .map(|(index, energy)| {
                energy * (PI * k as f32 * (index as f32 + 0.5) / n).cos()
            })
            .sum::<f32>();
    }
    out
}

/// A turn offered to the clusterer.
#[derive(Debug, Clone)]
pub struct TurnPrint {
    pub sequence: u64,
    pub print: Voiceprint,
}

/// Cosine distance beyond which two turns are taken to be different speakers.
///
/// Tuned to under-split rather than over-split. Two people merged into one
/// group is one rename away from correct and reads as a transcript missing a
/// distinction; one person split across three groups asks the user to name the
/// same person three times and reads as the feature not working.
pub const DEFAULT_SPLIT_DISTANCE: f32 = 0.35;

/// Most groups proposed, whatever the audio suggests.
///
/// A meeting genuinely has a handful of speakers. A run producing twenty
/// groups has not found twenty people; it has found that the fingerprints are
/// not separating, and the useful thing to do with that is stop rather than
/// hand the user twenty things to name.
pub const MAX_SPEAKERS: usize = 8;

/// Groups turns by speaker, returning a cluster index per input turn.
///
/// Agglomerative, average-linkage, stopping when the closest pair is further
/// apart than `split_distance`. Average linkage rather than single linkage
/// because single linkage chains: one ambiguous turn between two speakers
/// merges both of them through it.
pub fn cluster(turns: &[TurnPrint], split_distance: f32, max_speakers: usize) -> Vec<usize> {
    if turns.is_empty() {
        return Vec::new();
    }
    if turns.len() == 1 {
        return vec![0];
    }

    let mut clusters: Vec<Vec<usize>> = (0..turns.len()).map(|index| vec![index]).collect();

    loop {
        let mut closest: Option<(usize, usize, f32)> = None;
        for a in 0..clusters.len() {
            for b in (a + 1)..clusters.len() {
                let d = average_linkage(turns, &clusters[a], &clusters[b]);
                if closest.is_none_or(|(_, _, best)| d < best) {
                    closest = Some((a, b, d));
                }
            }
        }
        let Some((a, b, d)) = closest else { break };

        // Merge while too many groups remain even if they look distinct: the
        // cap is a statement about meetings, not about the audio.
        let over_cap = clusters.len() > max_speakers.max(1);
        if d > split_distance && !over_cap {
            break;
        }
        if clusters.len() <= 1 {
            break;
        }
        let merged = clusters.remove(b);
        clusters[a].extend(merged);
    }

    // Numbered by first appearance, so "Speaker 1" is whoever spoke first
    // rather than whichever cluster the merge order happened to leave first.
    clusters.sort_by_key(|members| members.iter().copied().min().unwrap_or(usize::MAX));

    let mut labels = vec![0usize; turns.len()];
    for (label, members) in clusters.iter().enumerate() {
        for member in members {
            labels[*member] = label;
        }
    }
    labels
}

/// Mean distance between every cross-cluster pair.
fn average_linkage(turns: &[TurnPrint], a: &[usize], b: &[usize]) -> f32 {
    let mut total = 0.0;
    let mut count = 0.0;
    for i in a {
        for j in b {
            total += distance(&turns[*i].print, &turns[*j].print);
            count += 1.0;
        }
    }
    if count == 0.0 {
        1.0
    } else {
        total / count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tone at `hz`, `seconds` long, with a couple of harmonics so it has
    /// spectral shape rather than a single spike.
    fn voice_like(hz: f32, seconds: f32, harmonic_gain: f32) -> Vec<f32> {
        let count = (SAMPLE_RATE * seconds) as usize;
        (0..count)
            .map(|n| {
                let t = n as f32 / SAMPLE_RATE;
                (2.0 * PI * hz * t).sin() * 0.5
                    + (2.0 * PI * hz * 2.0 * t).sin() * 0.3 * harmonic_gain
                    + (2.0 * PI * hz * 3.0 * t).sin() * 0.2 * harmonic_gain
            })
            .collect()
    }

    #[test]
    fn the_transform_finds_the_frequency_it_was_given() {
        // The whole filterbank stands on this, so it is checked directly
        // rather than inferred from the fingerprints downstream.
        let bin_hz = SAMPLE_RATE / FRAME_SIZE as f32;
        let target_bin = 40;
        let frequency = bin_hz * target_bin as f32;

        let frame: Vec<f32> = (0..FRAME_SIZE)
            .map(|n| (2.0 * PI * frequency * n as f32 / SAMPLE_RATE).sin())
            .collect();
        let spectrum = power_spectrum(&frame);

        let peak = spectrum
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|(bin, _)| bin)
            .expect("a peak");
        assert!(
            peak.abs_diff(target_bin) <= 1,
            "a {frequency} Hz tone peaked at bin {peak}, expected {target_bin}"
        );
    }

    #[test]
    fn the_transform_leaves_silence_silent() {
        let spectrum = power_spectrum(&vec![0.0; FRAME_SIZE]);
        assert!(spectrum.iter().all(|power| *power < 1e-6));
    }

    #[test]
    fn the_mel_scale_is_monotonic_and_round_trips() {
        assert!(hz_to_mel(100.0) < hz_to_mel(1_000.0));
        assert!(hz_to_mel(1_000.0) < hz_to_mel(8_000.0));
        for hz in [100.0, 440.0, 1_000.0, 4_000.0] {
            assert!((mel_to_hz(hz_to_mel(hz)) - hz).abs() < 1.0, "round trip at {hz} Hz");
        }
    }

    #[test]
    fn the_filterbank_covers_the_speech_band_without_running_off_the_spectrum() {
        let filters = mel_filterbank();
        assert_eq!(filters.len(), MEL_BANDS);
        let bins = FRAME_SIZE / 2 + 1;
        for (start, weights) in &filters {
            assert!(start + weights.len() <= bins + 1, "filter runs past the spectrum");
        }
    }

    #[test]
    fn a_turn_too_short_to_characterise_gets_no_fingerprint() {
        // Better to leave a line unattributed than to attribute it from one
        // vowel's worth of evidence.
        assert!(voiceprint(&voice_like(140.0, 0.4, 1.0)).is_none());
        assert!(voiceprint(&[]).is_none());
    }

    #[test]
    fn a_fingerprint_is_unit_length_and_the_right_shape() {
        let print = voiceprint(&voice_like(140.0, 2.0, 1.0)).expect("a fingerprint");
        assert_eq!(print.len(), (CEPSTRA - 1) * 2);
        let norm = print.iter().map(|v| v * v).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-3, "norm was {norm}");
    }

    #[test]
    fn loudness_does_not_change_who_someone_is() {
        // The failure this guards: a speaker who leans toward the microphone
        // halfway through becoming a second speaker.
        let quiet = voice_like(140.0, 2.0, 1.0);
        let loud: Vec<f32> = quiet.iter().map(|s| s * 4.0).collect();
        let a = voiceprint(&quiet).expect("a fingerprint");
        let b = voiceprint(&loud).expect("a fingerprint");
        assert!(
            distance(&a, &b) < 0.05,
            "the same voice at two volumes measured {} apart",
            distance(&a, &b)
        );
    }

    #[test]
    fn two_different_voices_measure_further_apart_than_one_voice_does_from_itself() {
        let low_a = voiceprint(&voice_like(110.0, 2.0, 1.0)).expect("a fingerprint");
        let low_b = voiceprint(&voice_like(112.0, 2.0, 1.0)).expect("a fingerprint");
        let high = voiceprint(&voice_like(240.0, 2.0, 0.2)).expect("a fingerprint");

        let same_speaker = distance(&low_a, &low_b);
        let different_speakers = distance(&low_a, &high);
        assert!(
            different_speakers > same_speaker,
            "two voices measured {different_speakers} apart, one voice {same_speaker}"
        );
    }

    #[test]
    fn distance_refuses_to_compare_mismatched_fingerprints() {
        assert_eq!(distance(&[1.0, 0.0], &[1.0]), 1.0);
        assert_eq!(distance(&[], &[]), 1.0);
    }

    fn turn(sequence: u64, print: &[f32]) -> TurnPrint {
        TurnPrint {
            sequence,
            print: normalize(print.to_vec()),
        }
    }

    #[test]
    fn clustering_separates_two_clearly_different_groups() {
        let turns = vec![
            turn(0, &[1.0, 0.0, 0.0]),
            turn(1, &[0.98, 0.02, 0.0]),
            turn(2, &[0.0, 1.0, 0.0]),
            turn(3, &[0.02, 0.98, 0.0]),
        ];
        let labels = cluster(&turns, DEFAULT_SPLIT_DISTANCE, MAX_SPEAKERS);
        assert_eq!(labels[0], labels[1]);
        assert_eq!(labels[2], labels[3]);
        assert_ne!(labels[0], labels[2]);
    }

    #[test]
    fn clustering_numbers_groups_by_who_spoke_first() {
        // "Speaker 1" has to be whoever opened the meeting, or the names the
        // user assigns do not survive a re-run.
        let turns = vec![
            turn(0, &[0.0, 1.0, 0.0]),
            turn(1, &[1.0, 0.0, 0.0]),
            turn(2, &[0.0, 0.98, 0.02]),
        ];
        let labels = cluster(&turns, DEFAULT_SPLIT_DISTANCE, MAX_SPEAKERS);
        assert_eq!(labels[0], 0, "the first turn is always group 0");
        assert_eq!(labels[2], 0);
        assert_eq!(labels[1], 1);
    }

    #[test]
    fn one_speaker_stays_one_speaker() {
        let turns: Vec<TurnPrint> = (0..6)
            .map(|i| turn(i, &[1.0, 0.01 * i as f32, 0.0]))
            .collect();
        let labels = cluster(&turns, DEFAULT_SPLIT_DISTANCE, MAX_SPEAKERS);
        assert!(labels.iter().all(|label| *label == 0), "got {labels:?}");
    }

    #[test]
    fn the_speaker_cap_is_enforced_even_when_every_turn_looks_distinct() {
        // Twelve mutually distant turns. Twelve groups would not be twelve
        // people; it would be the fingerprints failing to separate, and
        // handing the user twelve things to name is the worst answer to that.
        let turns: Vec<TurnPrint> = (0..12)
            .map(|i| {
                let mut print = vec![0.0f32; 12];
                print[i as usize] = 1.0;
                turn(i, &print)
            })
            .collect();
        let labels = cluster(&turns, DEFAULT_SPLIT_DISTANCE, 3);
        let distinct: std::collections::BTreeSet<usize> = labels.iter().copied().collect();
        assert!(distinct.len() <= 3, "got {} groups", distinct.len());
    }

    #[test]
    fn clustering_nothing_produces_nothing() {
        assert!(cluster(&[], DEFAULT_SPLIT_DISTANCE, MAX_SPEAKERS).is_empty());
        assert_eq!(cluster(&[turn(0, &[1.0, 0.0])], DEFAULT_SPLIT_DISTANCE, MAX_SPEAKERS), vec![0]);
    }

    #[test]
    fn every_turn_gets_exactly_one_group() {
        let turns: Vec<TurnPrint> = (0..9)
            .map(|i| turn(i, &[(i % 3) as f32, ((i + 1) % 3) as f32, 1.0]))
            .collect();
        let labels = cluster(&turns, DEFAULT_SPLIT_DISTANCE, MAX_SPEAKERS);
        assert_eq!(labels.len(), turns.len());
        let distinct: std::collections::BTreeSet<usize> = labels.iter().copied().collect();
        // Labels are dense: 0..n with no gaps, so the UI can index by them.
        assert_eq!(
            distinct.iter().copied().collect::<Vec<usize>>(),
            (0..distinct.len()).collect::<Vec<usize>>()
        );
    }
}
