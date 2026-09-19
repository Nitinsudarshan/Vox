//! Speaking while the answer is still being written, and stopping mid-sentence.
//!
//! ## One consumer, so ordering is structural
//!
//! Phrases are synthesized by a single worker thread reading a bounded
//! channel. One consumer means they come out in the order they went in as a
//! property of the design rather than as a sort, which is the same argument
//! the meeting decoder makes and it holds for the same reason.
//!
//! Bounded because a model that runs away must not grow a backlog of audio
//! nobody will hear. Past the ceiling a phrase is refused and counted — the
//! same discipline as the decode queue, and for the same reason: a bounded
//! system that sheds load invisibly is worse than an unbounded one.
//!
//! ## Cancellation, and why it is not a flag between items
//!
//! Stopping has to mean stopping *now* — because the user asked, because they
//! started talking, because the answer turned out to be wrong. A design that
//! checks a flag between utterances cannot be made interruptible later; the
//! longest thing it ever says becomes the worst case, and that is the case
//! that matters.
//!
//! So the token is shared with the provider and the sink, and everything
//! already queued is dropped rather than drained. Audio that arrives for a
//! turn the user has moved on from is thrown away, not played late.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc as std_mpsc;
use std::sync::Arc;

use super::{SynthesizedAudio, TextToSpeech, TtsError, Utterance};

/// Phrases the queue holds before it refuses one.
///
/// Twenty-four is several sentences ahead of playback — deep enough that a
/// slow synthesizer never makes the speaker stutter, shallow enough that a
/// runaway model is stopped rather than buffered.
pub const MAX_QUEUED_PHRASES: usize = 24;

/// Where finished audio goes.
///
/// A trait so ordering, cancellation, "no phrase twice" and "no phrase
/// dropped" can be tested against a recording sink with no audio device
/// anywhere in sight. Playback itself is the frontend's: the browser gives
/// ordered queueing and instant cancellation for free, and putting a second
/// audio stack in the process to avoid that would be a strange trade.
pub trait SpeechSink: Send {
    /// One phrase's audio, in the order it was queued.
    fn accept(&mut self, sequence: u64, audio: SynthesizedAudio);

    /// A phrase that could not be synthesized.
    ///
    /// Reported rather than skipped: a gap in spoken output with no
    /// explanation is indistinguishable from the answer having ended.
    fn failed(&mut self, sequence: u64, error: TtsError);
}

/// What a speaking turn did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpeechStats {
    pub queued: u64,
    pub spoken: u64,
    pub failed: u64,
    /// Refused because the queue was full.
    pub dropped: u64,
    /// Abandoned because the turn was cancelled.
    pub cancelled: u64,
}

/// A speaking turn in flight.
pub struct SpeechQueue {
    tx: Option<std_mpsc::SyncSender<(u64, Utterance)>>,
    worker: Option<std::thread::JoinHandle<SpeechStats>>,
    cancel: Arc<AtomicBool>,
    next_sequence: AtomicU64,
    dropped: Arc<AtomicU64>,
}

impl SpeechQueue {
    /// Starts a speaking turn.
    ///
    /// The provider is moved onto the worker thread, so synthesis never runs
    /// on whatever is producing the text — which for a streamed answer is the
    /// thread feeding this queue.
    pub fn start(provider: Arc<dyn TextToSpeech>, mut sink: Box<dyn SpeechSink>) -> Self {
        let (tx, rx) = std_mpsc::sync_channel::<(u64, Utterance)>(MAX_QUEUED_PHRASES);
        let cancel = Arc::new(AtomicBool::new(false));
        let dropped = Arc::new(AtomicU64::new(0));

        let worker_cancel = Arc::clone(&cancel);
        let worker_dropped = Arc::clone(&dropped);
        let worker = std::thread::Builder::new()
            .name("vox-tts-synthesize".into())
            .spawn(move || {
                let mut stats = SpeechStats::default();
                for (sequence, utterance) in rx {
                    stats.queued += 1;
                    if worker_cancel.load(Ordering::SeqCst) {
                        stats.cancelled += 1;
                        continue;
                    }
                    match provider.synthesize(&utterance) {
                        Ok(audio) => {
                            // Checked again: synthesis takes time, and the
                            // user may have moved on during it. Audio for a
                            // turn that is over is thrown away, not played
                            // late.
                            if worker_cancel.load(Ordering::SeqCst) {
                                stats.cancelled += 1;
                                continue;
                            }
                            stats.spoken += 1;
                            sink.accept(sequence, audio);
                        }
                        Err(error) => {
                            stats.failed += 1;
                            sink.failed(sequence, error);
                        }
                    }
                }
                stats.dropped = worker_dropped.load(Ordering::SeqCst);
                stats
            })
            .expect("spawning the speech synthesis thread");

        Self {
            tx: Some(tx),
            worker: Some(worker),
            cancel,
            next_sequence: AtomicU64::new(0),
            dropped,
        }
    }

    /// Queues one phrase. Returns whether it was accepted.
    ///
    /// Never blocks. The caller is usually inside a model's streaming
    /// callback, and blocking there would stall the generation that is
    /// producing the next phrase — which is the thing this design exists to
    /// overlap.
    pub fn speak(&self, utterance: Utterance) -> bool {
        if self.cancel.load(Ordering::SeqCst) {
            return false;
        }
        let Some(tx) = self.tx.as_ref() else {
            return false;
        };
        let sequence = self.next_sequence.fetch_add(1, Ordering::SeqCst);
        match tx.try_send((sequence, utterance)) {
            Ok(()) => true,
            Err(_) => {
                self.dropped.fetch_add(1, Ordering::SeqCst);
                false
            }
        }
    }

    /// Stops the turn now.
    ///
    /// Everything queued is abandoned and anything mid-synthesis is discarded
    /// when it finishes. Idempotent, because the reasons to cancel — a click,
    /// a barge-in, a new question — can easily arrive together.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    /// A token the provider can watch, so a long synthesis can abandon itself
    /// rather than finishing work nobody will hear.
    pub fn cancel_token(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancel)
    }

    /// Ends the turn and waits for the worker.
    pub fn finish(mut self) -> SpeechStats {
        drop(self.tx.take());
        self.worker
            .take()
            .map(|handle| handle.join().unwrap_or_default())
            .unwrap_or_default()
    }
}

impl Drop for SpeechQueue {
    fn drop(&mut self) {
        // A dropped queue is an abandoned turn. Cancelling first means the
        // worker stops rather than synthesizing a backlog for a sink that is
        // about to go away.
        self.cancel();
        drop(self.tx.take());
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tts::{TtsCapabilities, TtsDescriptor};
    use std::sync::Mutex;
    use std::time::Duration;

    /// A provider that returns scripted audio after a scripted delay.
    struct FakeTts {
        delay: Duration,
        fail_on: Option<String>,
    }

    impl FakeTts {
        fn instant() -> Arc<Self> {
            Arc::new(Self {
                delay: Duration::ZERO,
                fail_on: None,
            })
        }
    }

    impl TextToSpeech for FakeTts {
        fn descriptor(&self) -> TtsDescriptor {
            TtsDescriptor {
                id: "fake".into(),
                display_name: "Fake".into(),
                capabilities: TtsCapabilities::none(),
                voices: Vec::new(),
            }
        }

        fn is_available(&self) -> bool {
            true
        }

        fn synthesize(&self, utterance: &Utterance) -> Result<SynthesizedAudio, TtsError> {
            if !self.delay.is_zero() {
                std::thread::sleep(self.delay);
            }
            if self.fail_on.as_deref() == Some(utterance.text.as_str()) {
                return Err(TtsError::Failed("that one broke".into()));
            }
            Ok(SynthesizedAudio {
                bytes: utterance.text.as_bytes().to_vec(),
                media_type: "audio/wav".into(),
                duration_ms: None,
            })
        }
    }

    #[derive(Default)]
    struct Recorded {
        accepted: Vec<(u64, String)>,
        failures: Vec<u64>,
    }

    struct RecordingSink(Arc<Mutex<Recorded>>);

    impl SpeechSink for RecordingSink {
        fn accept(&mut self, sequence: u64, audio: SynthesizedAudio) {
            self.0
                .lock()
                .unwrap()
                .accepted
                .push((sequence, String::from_utf8(audio.bytes).unwrap()));
        }

        fn failed(&mut self, sequence: u64, _error: TtsError) {
            self.0.lock().unwrap().failures.push(sequence);
        }
    }

    fn sink() -> (Arc<Mutex<Recorded>>, Box<dyn SpeechSink>) {
        let recorded = Arc::new(Mutex::new(Recorded::default()));
        (Arc::clone(&recorded), Box::new(RecordingSink(recorded)))
    }

    #[test]
    fn a_short_answer_is_spoken_once_and_in_order() {
        let (recorded, sink) = sink();
        let queue = SpeechQueue::start(FakeTts::instant(), sink);
        for text in ["first", "second", "third"] {
            assert!(queue.speak(Utterance::new(text)));
        }
        let stats = queue.finish();

        assert_eq!(stats.spoken, 3);
        let recorded = recorded.lock().unwrap();
        assert_eq!(
            recorded.accepted,
            vec![
                (0, "first".to_string()),
                (1, "second".to_string()),
                (2, "third".to_string())
            ]
        );
    }

    #[test]
    fn a_long_answer_keeps_its_order_through_a_slow_synthesizer() {
        // One consumer, so ordering is a property of the design rather than a
        // sort applied afterwards.
        let (recorded, sink) = sink();
        let provider = Arc::new(FakeTts {
            delay: Duration::from_millis(2),
            fail_on: None,
        });
        let queue = SpeechQueue::start(provider, sink);
        for index in 0..12 {
            queue.speak(Utterance::new(format!("phrase {index}")));
        }
        let stats = queue.finish();

        assert_eq!(stats.spoken, 12);
        let recorded = recorded.lock().unwrap();
        let sequences: Vec<u64> = recorded.accepted.iter().map(|(seq, _)| *seq).collect();
        assert_eq!(sequences, (0..12).collect::<Vec<_>>());
    }

    #[test]
    fn cancelling_stops_the_turn_rather_than_draining_it() {
        // The requirement: stopping means stopping now. A design that drains
        // makes the longest thing it ever says the worst case, and that is
        // the case that matters.
        let (recorded, sink) = sink();
        let provider = Arc::new(FakeTts {
            delay: Duration::from_millis(20),
            fail_on: None,
        });
        let queue = SpeechQueue::start(provider, sink);
        for index in 0..10 {
            queue.speak(Utterance::new(format!("phrase {index}")));
        }
        queue.cancel();
        let stats = queue.finish();

        assert!(queue_was_cut_short(&stats), "{stats:?}");
        assert!(recorded.lock().unwrap().accepted.len() < 10);
    }

    fn queue_was_cut_short(stats: &SpeechStats) -> bool {
        stats.cancelled > 0 && stats.spoken < stats.queued
    }

    #[test]
    fn speaking_after_cancellation_is_refused_rather_than_queued() {
        let (recorded, sink) = sink();
        let queue = SpeechQueue::start(FakeTts::instant(), sink);
        queue.cancel();
        assert!(!queue.speak(Utterance::new("too late")));
        let stats = queue.finish();
        assert_eq!(stats.spoken, 0);
        assert!(recorded.lock().unwrap().accepted.is_empty());
    }

    #[test]
    fn cancelling_twice_is_the_same_as_cancelling_once() {
        // A click, a barge-in and a new question can easily arrive together.
        let (_, sink) = sink();
        let queue = SpeechQueue::start(FakeTts::instant(), sink);
        queue.cancel();
        queue.cancel();
        assert!(queue.is_cancelled());
        let _ = queue.finish();
    }

    #[test]
    fn a_phrase_that_will_not_synthesize_is_reported_not_skipped() {
        // A gap in spoken output with no explanation is indistinguishable
        // from the answer having ended.
        let (recorded, sink) = sink();
        let provider = Arc::new(FakeTts {
            delay: Duration::ZERO,
            fail_on: Some("broken".into()),
        });
        let queue = SpeechQueue::start(provider, sink);
        queue.speak(Utterance::new("fine"));
        queue.speak(Utterance::new("broken"));
        queue.speak(Utterance::new("also fine"));
        let stats = queue.finish();

        assert_eq!(stats.failed, 1);
        assert_eq!(stats.spoken, 2);
        assert_eq!(recorded.lock().unwrap().failures, vec![1]);
    }

    #[test]
    fn a_runaway_model_is_refused_rather_than_buffered() {
        // Bounded, and the refusal is counted — a bounded system that sheds
        // load invisibly is worse than an unbounded one.
        let (_, sink) = sink();
        let provider = Arc::new(FakeTts {
            delay: Duration::from_millis(50),
            fail_on: None,
        });
        let queue = SpeechQueue::start(provider, sink);
        let mut refused = 0;
        for index in 0..(MAX_QUEUED_PHRASES * 4) {
            if !queue.speak(Utterance::new(format!("phrase {index}"))) {
                refused += 1;
            }
        }
        queue.cancel();
        let stats = queue.finish();
        assert!(refused > 0, "the queue must have a ceiling");
        assert!(stats.dropped > 0, "and must count what it refused");
    }

    #[test]
    fn queueing_never_blocks_the_thread_producing_the_text() {
        // The caller is usually inside a model's streaming callback, and
        // blocking there stalls the generation producing the next phrase.
        let (_, sink) = sink();
        let provider = Arc::new(FakeTts {
            delay: Duration::from_millis(30),
            fail_on: None,
        });
        let queue = SpeechQueue::start(provider, sink);
        let started = std::time::Instant::now();
        for index in 0..(MAX_QUEUED_PHRASES * 3) {
            queue.speak(Utterance::new(format!("phrase {index}")));
        }
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "queueing took {:?}",
            started.elapsed()
        );
        queue.cancel();
        let _ = queue.finish();
    }

    #[test]
    fn dropping_a_turn_cancels_it_rather_than_synthesizing_for_nobody() {
        let (recorded, sink) = sink();
        let provider = Arc::new(FakeTts {
            delay: Duration::from_millis(20),
            fail_on: None,
        });
        {
            let queue = SpeechQueue::start(provider, sink);
            for index in 0..10 {
                queue.speak(Utterance::new(format!("phrase {index}")));
            }
        }
        assert!(recorded.lock().unwrap().accepted.len() < 10);
    }

    #[test]
    fn a_turn_that_says_nothing_finishes_cleanly() {
        let (_, sink) = sink();
        let queue = SpeechQueue::start(FakeTts::instant(), sink);
        assert_eq!(queue.finish(), SpeechStats::default());
    }
}
