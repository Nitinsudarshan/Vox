//! Recording both sides of a conversation.
//!
//! Two `cpal` input streams — the microphone, and the default *output* device
//! opened in loopback so whatever the machine is playing is captured too —
//! drained in temporal lockstep and soft-mixed into one 16 kHz mono stream.
//!
//! ## Lockstep, and why it is not "take whatever is available"
//!
//! With both streams live the mixer consumes the same number of samples from
//! each, so `mic[t]` is only ever mixed with `sys[t]`. Consuming `max` and
//! zero-padding whichever stream is momentarily behind — which is what
//! meetily's ring buffer does — mixes real samples against padding, and then
//! mixes the late arrivals against *future* audio from the other stream. The
//! two sides of the call drift further apart for the rest of the meeting.
//!
//! Strict lockstep alone would stall, because WASAPI loopback delivers no
//! callbacks at all while nothing is playing. Once one stream is more than
//! [`MAX_STREAM_LAG_SAMPLES`] ahead the mixer advances anyway and pads the
//! silent side, which caps misalignment at that lag instead of letting it
//! accumulate.
//!
//! ## Per-channel energy
//!
//! The mixer keeps each channel's energy alongside the mixed samples. That is
//! the whole basis for a transcript line saying "You" or "Others": mix first
//! and the information is gone, which is the position meetily is in.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use tauri::{AppHandle, Emitter};

use crate::sync::MutexExt;

use super::segmenter::SEGMENT_SAMPLE_RATE;

/// How often the mixer wakes to drain the device FIFOs.
const MIXER_TICK: Duration = Duration::from_millis(20);

/// How often a level reading reaches the UI. 25 Hz is smooth to the eye and
/// cheap enough that the meter is never why a frame is late.
const LEVEL_EMIT_INTERVAL: Duration = Duration::from_millis(40);

/// Exponential smoothing applied to the level meters.
const LEVEL_SMOOTHING_ALPHA: f32 = 0.35;

/// How far one stream may run ahead of a silent one before the mixer stops
/// waiting. 250 ms: long enough that ordinary jitter never triggers it, short
/// enough that the pad it inserts is not audible as a gap.
const MAX_STREAM_LAG_SAMPLES: usize = (SEGMENT_SAMPLE_RATE as f64 * 0.25) as usize;

/// Mixed blocks the channel to the pump holds before the mixer sheds one.
///
/// Ten seconds at [`MIXER_TICK`]. The pump segments and writes checkpoints, so
/// a disk stall of a second or two is ordinary and must cost nothing; ten
/// seconds of slack absorbs that and still bounds memory.
///
/// Unbounded was the previous answer and it is the worse one. A pump that
/// stalls indefinitely against an unbounded channel grows memory until the
/// process dies, which loses every second of the meeting after the last
/// checkpoint. Shedding twenty milliseconds and counting it loses twenty
/// milliseconds. Neither is good; only one of them is bounded.
const AUDIO_CHANNEL_BLOCKS: usize = 500;

/// Samples one device FIFO holds before its oldest are discarded.
///
/// A minute. The mixer drains both FIFOs every tick and never blocks, so
/// reaching this means the mixer thread itself is not running — and at that
/// point the choice is between a bounded loss and an unbounded one.
const MAX_FIFO_SAMPLES: usize = SEGMENT_SAMPLE_RATE as usize * 60;

/// RMS under which a block is treated as silence for the meter and for
/// "did this source ever carry anything".
const AUDIBLE_RMS_THRESHOLD: f32 = 0.004;

/// The decibel window the level meter maps onto bar height. Conversational
/// speech sits around −34 dBFS, so it lands mid-bar rather than against an end.
const METER_FLOOR_DB: f32 = -55.0;
const METER_CEILING_DB: f32 = -15.0;

/// Live microphone and system-audio levels, for the recording surface.
pub const MEETING_LEVEL_EVENT: &str = "meeting-audio-level";

/// Something went wrong with the *recording*, as distinct from with the
/// transcript.
///
/// A separate event from `meeting-transcription-warning` because they are
/// different failures with different remedies, and because the audio one is
/// the more serious of the two: a transcript can be regenerated from a
/// recording and a recording cannot be regenerated from anything.
pub const MEETING_RECORDING_WARNING_EVENT: &str = "meeting-recording-warning";

/// What the recorder could not do.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RecordingWarning {
    /// `microphone`, `system_audio`, `audio_storage` or `audio_shed`.
    pub kind: String,
    pub message: String,
}

/// Emits a recording warning, if there is a window to emit it to.
pub fn emit_recording_warning(app: &Option<AppHandle>, kind: &str, message: &str) {
    tracing::warn!("meeting recording warning [{}]: {}", kind, message);
    if let Some(handle) = app {
        let _ = handle.emit(
            MEETING_RECORDING_WARNING_EVENT,
            RecordingWarning {
                kind: kind.to_string(),
                message: message.to_string(),
            },
        );
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct MeetingLevels {
    pub mic: f32,
    pub system: f32,
}

/// Which devices one recording should open.
///
/// `None` on either side means "resolve it the way every other surface does" —
/// the shared microphone preference, and the OS default output. A name that no
/// longer matches anything falls back the same way rather than failing the
/// recording: a user who unplugs the headset they recorded with last week
/// wants to be recorded, not to be right about the device.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MeetingDevices {
    /// Microphone, by the name `get_audio_devices` reports.
    #[serde(default)]
    pub microphone: Option<String>,
    /// The output device to capture in loopback — what this machine plays.
    #[serde(default)]
    pub system_audio: Option<String>,
}

impl MeetingDevices {
    /// Whether either side names a device.
    pub fn is_empty(&self) -> bool {
        self.microphone.is_none() && self.system_audio.is_none()
    }
}

/// What was actually opened, so the surface can name it.
///
/// The recorder already reports whether each channel has *heard* anything,
/// which is how a wrong device is noticed. Saying which device that is turns
/// "the microphone bar never moved" into something the user can act on without
/// opening system settings to guess.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct OpenedDevices {
    pub microphone: Option<String>,
    pub system_audio: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum MeetingCaptureError {
    #[error("no microphone or system audio device could be opened")]
    NoDevice,

    #[error("audio capture thread failed to start: {0}")]
    StartFailed(String),
}

/// One mixer tick's worth of audio, at [`SEGMENT_SAMPLE_RATE`].
///
/// `mic` and `sys` are the same span before mixing, so a consumer can
/// attribute it to a channel. Both are the same length as `mixed`.
#[derive(Debug, Clone)]
pub struct MixedAudio {
    pub mixed: Vec<f32>,
    pub mic: Vec<f32>,
    pub sys: Vec<f32>,
    /// Set on the first block after a resume: the audio before it and the
    /// audio after it are not contiguous in time, so a decoder should not read
    /// across the join.
    pub discontinuity: bool,
}

/// What was actually bound when capture started.
#[derive(Debug, Clone, Default)]
pub struct CaptureBinding {
    pub microphone: bool,
    pub system_audio: bool,
    /// The device names behind those flags.
    pub opened: OpenedDevices,
}

/// A running dual-stream capture.
///
/// The `cpal` streams are not `Send`, so they live on the mixer thread and are
/// dropped there; this handle only carries flags and the stop signal.
pub struct DualCapture {
    stop_tx: std_mpsc::Sender<()>,
    paused: Arc<AtomicBool>,
    mic_active: Arc<AtomicBool>,
    sys_active: Arc<AtomicBool>,
    mic_heard: Arc<AtomicBool>,
    sys_heard: Arc<AtomicBool>,
    losses: Arc<AudioLosses>,
    binding: CaptureBinding,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// Audio that was captured and did not reach the pump.
///
/// Both counters should be zero on every ordinary recording. They exist so
/// that when they are not, the recording says so instead of quietly having a
/// hole in it.
#[derive(Debug, Default)]
pub struct AudioLosses {
    /// Mixed blocks dropped because the channel to the pump was full.
    pub blocks_shed: AtomicU64,
    /// Samples discarded from a device FIFO that the mixer stopped draining.
    pub fifo_samples_dropped: AtomicU64,
}

impl AudioLosses {
    /// Seconds of audio lost, across both causes.
    pub fn lost_seconds(&self, block_samples: u64) -> f64 {
        let shed = self.blocks_shed.load(Ordering::SeqCst) * block_samples;
        let fifo = self.fifo_samples_dropped.load(Ordering::SeqCst);
        (shed + fifo) as f64 / SEGMENT_SAMPLE_RATE as f64
    }

    pub fn any(&self) -> bool {
        self.blocks_shed.load(Ordering::SeqCst) > 0
            || self.fifo_samples_dropped.load(Ordering::SeqCst) > 0
    }
}

impl DualCapture {
    /// Opens both streams and starts mixing.
    ///
    /// Returns the handle and the channel mixed audio arrives on. Fails only
    /// when neither device could be opened — a missing system-audio device is
    /// reported through [`CaptureBinding`] and the recording proceeds with the
    /// microphone alone, because half a meeting is worth far more than none.
    pub fn start(
        app: Option<AppHandle>,
        capture_system_audio: bool,
        devices: MeetingDevices,
    ) -> Result<(Self, std_mpsc::Receiver<MixedAudio>), MeetingCaptureError> {
        let (stop_tx, stop_rx) = std_mpsc::channel();
        // Bounded: see [`AUDIO_CHANNEL_BLOCKS`]. The mixer never blocks on it,
        // so a stalled pump cannot back up into the device FIFOs either.
        let (audio_tx, audio_rx) = std_mpsc::sync_channel(AUDIO_CHANNEL_BLOCKS);
        let (init_tx, init_rx) = std_mpsc::channel();

        let paused = Arc::new(AtomicBool::new(false));
        let mic_active = Arc::new(AtomicBool::new(false));
        let sys_active = Arc::new(AtomicBool::new(false));
        let mic_heard = Arc::new(AtomicBool::new(false));
        let sys_heard = Arc::new(AtomicBool::new(false));
        let losses = Arc::new(AudioLosses::default());

        let ctx = LoopContext {
            paused: paused.clone(),
            mic_active: mic_active.clone(),
            sys_active: sys_active.clone(),
            mic_heard: mic_heard.clone(),
            sys_heard: sys_heard.clone(),
            losses: Arc::clone(&losses),
            app,
            capture_system_audio,
            devices,
        };

        let thread = std::thread::Builder::new()
            .name("vox-meeting-capture".into())
            .spawn(move || run_capture_loop(ctx, stop_rx, audio_tx, init_tx))
            .map_err(|err| MeetingCaptureError::StartFailed(err.to_string()))?;

        // Wait for the thread to report what it bound, so a caller never
        // believes a recording started when no device opened.
        let binding = match init_rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(binding)) => binding,
            Ok(Err(())) => return Err(MeetingCaptureError::NoDevice),
            Err(err) => return Err(MeetingCaptureError::StartFailed(err.to_string())),
        };

        Ok((
            Self {
                stop_tx,
                paused,
                mic_active,
                sys_active,
                mic_heard,
                sys_heard,
                losses,
                binding,
                thread: Some(thread),
            },
            audio_rx,
        ))
    }

    pub fn binding(&self) -> CaptureBinding {
        self.binding.clone()
    }

    /// Whether each stream is still delivering callbacks. A stream that errors
    /// out clears its flag rather than being reported as live forever.
    pub fn is_microphone_active(&self) -> bool {
        self.mic_active.load(Ordering::SeqCst)
    }

    pub fn is_system_audio_active(&self) -> bool {
        self.sys_active.load(Ordering::SeqCst)
    }

    /// Whether anything above the silence floor ever arrived on that channel.
    pub fn microphone_heard(&self) -> bool {
        self.mic_heard.load(Ordering::SeqCst)
    }

    pub fn system_audio_heard(&self) -> bool {
        self.sys_heard.load(Ordering::SeqCst)
    }

    /// Audio that was captured and never reached the pump. Zero on every
    /// ordinary recording.
    pub fn losses(&self) -> Arc<AudioLosses> {
        Arc::clone(&self.losses)
    }

    /// Discards incoming audio until [`DualCapture::resume`].
    ///
    /// Paused time is excised from the recording rather than stored as
    /// silence, so a meeting's audio length matches the speech in it.
    pub fn pause(&self) {
        self.paused.store(true, Ordering::SeqCst);
    }

    pub fn resume(&self) {
        self.paused.store(false, Ordering::SeqCst);
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }

    /// Stops both streams and waits for the mixer thread to finish.
    ///
    /// Blocking on purpose: the caller's next step is merging the audio the
    /// thread is still writing.
    pub fn stop(&mut self) {
        let _ = self.stop_tx.send(());
        if let Some(handle) = self.thread.take() {
            if let Err(err) = handle.join() {
                tracing::warn!("meeting capture thread panicked on shutdown: {:?}", err);
            }
        }
    }
}

impl Drop for DualCapture {
    fn drop(&mut self) {
        self.stop();
    }
}

struct LoopContext {
    paused: Arc<AtomicBool>,
    mic_active: Arc<AtomicBool>,
    sys_active: Arc<AtomicBool>,
    mic_heard: Arc<AtomicBool>,
    sys_heard: Arc<AtomicBool>,
    losses: Arc<AudioLosses>,
    app: Option<AppHandle>,
    capture_system_audio: bool,
    devices: MeetingDevices,
}

fn run_capture_loop(
    ctx: LoopContext,
    stop_rx: std_mpsc::Receiver<()>,
    audio_tx: std_mpsc::SyncSender<MixedAudio>,
    init_tx: std_mpsc::Sender<Result<CaptureBinding, ()>>,
) {
    let host = cpal::default_host();

    // Separate FIFOs so the two streams can be consumed in temporal lockstep.
    let mic_fifo: Arc<Mutex<VecDeque<f32>>> = Arc::new(Mutex::new(VecDeque::with_capacity(32_000)));
    let sys_fifo: Arc<Mutex<VecDeque<f32>>> = Arc::new(Mutex::new(VecDeque::with_capacity(32_000)));

    let mic_device = resolve_input_device(&host, ctx.devices.microphone.as_deref());
    let sys_device = if ctx.capture_system_audio {
        resolve_output_device(&host, ctx.devices.system_audio.as_deref())
    } else {
        None
    };

    let opened = OpenedDevices {
        microphone: mic_device.as_ref().and_then(|d| d.name().ok()),
        system_audio: sys_device.as_ref().and_then(|d| d.name().ok()),
    };

    let mic_stream = build_stream(mic_device, false, &mic_fifo, &ctx.mic_active, &ctx.losses);
    let sys_stream = build_stream(sys_device, true, &sys_fifo, &ctx.sys_active, &ctx.losses);

    let has_mic = mic_stream.is_some();
    let has_sys = sys_stream.is_some();

    if !has_mic && !has_sys {
        let _ = init_tx.send(Err(()));
        return;
    }
    let _ = init_tx.send(Ok(CaptureBinding {
        microphone: has_mic,
        system_audio: has_sys,
        opened: OpenedDevices {
            microphone: has_mic.then_some(opened.microphone).flatten(),
            system_audio: has_sys.then_some(opened.system_audio).flatten(),
        },
    }));

    let mut mic_level = 0.0f32;
    let mut sys_level = 0.0f32;
    let mut last_level_emit = Instant::now();
    let mut was_paused = false;
    let mut pending_discontinuity = false;

    loop {
        let stopping = stop_rx.try_recv().is_ok();

        if ctx.paused.load(Ordering::SeqCst) {
            drain_fifo(&mic_fifo);
            drain_fifo(&sys_fifo);
            if !was_paused {
                was_paused = true;
                pending_discontinuity = true;
                mic_level = 0.0;
                sys_level = 0.0;
                emit_levels(&ctx.app, 0.0, 0.0);
                last_level_emit = Instant::now();
            }
            if stopping {
                break;
            }
            std::thread::sleep(MIXER_TICK);
            continue;
        }
        was_paused = false;

        let (mixed, mic, sys, mic_sum_sq, sys_sum_sq) = {
            let mut mic_guard = mic_fifo.lock_or_recover();
            let mut sys_guard = sys_fifo.lock_or_recover();
            let count = plan_drain(mic_guard.len(), sys_guard.len(), has_mic, has_sys);

            let mut mixed = Vec::with_capacity(count);
            let mut mic = Vec::with_capacity(count);
            let mut sys = Vec::with_capacity(count);
            let mut mic_sq = 0.0f32;
            let mut sys_sq = 0.0f32;
            for _ in 0..count {
                let m = mic_guard.pop_front().unwrap_or(0.0);
                let s = sys_guard.pop_front().unwrap_or(0.0);
                mic_sq += m * m;
                sys_sq += s * s;
                mixed.push(soft_mix(m, s));
                mic.push(m);
                sys.push(s);
            }
            (mixed, mic, sys, mic_sq, sys_sq)
        };

        let drained = mixed.len();
        if drained > 0 {
            mark_heard(&ctx.mic_heard, mic_sum_sq, drained);
            mark_heard(&ctx.sys_heard, sys_sum_sq, drained);

            let mic_target = meter_level(rms_from_sum_sq(mic_sum_sq, drained));
            let sys_target = meter_level(rms_from_sum_sq(sys_sum_sq, drained));
            mic_level += (mic_target - mic_level) * LEVEL_SMOOTHING_ALPHA;
            sys_level += (sys_target - sys_level) * LEVEL_SMOOTHING_ALPHA;

            let block = MixedAudio {
                mixed,
                mic,
                sys,
                discontinuity: pending_discontinuity,
            };
            pending_discontinuity = false;
            match audio_tx.try_send(block) {
                Ok(()) => {}
                Err(std_mpsc::TrySendError::Full(block)) => {
                    // The pump has not drained for ten seconds. Blocking here
                    // would push the backlog into the device FIFOs instead,
                    // so the block is shed and counted — and the
                    // discontinuity flag it was carrying is handed to the next
                    // block, because the join it marks is still real.
                    pending_discontinuity = block.discontinuity;
                    let shed = ctx.losses.blocks_shed.fetch_add(1, Ordering::SeqCst) + 1;
                    if shed == 1 {
                        emit_recording_warning(
                            &ctx.app,
                            "audio_shed",
                            "Audio is arriving faster than it can be written to disk, so some \
                             of the recording is being lost. Recording continues.",
                        );
                    }
                }
                // A closed receiver means the engine is gone; there is nothing
                // left to record for.
                Err(std_mpsc::TrySendError::Disconnected(_)) => break,
            }
        }

        if last_level_emit.elapsed() >= LEVEL_EMIT_INTERVAL {
            last_level_emit = Instant::now();
            emit_levels(&ctx.app, mic_level, sys_level);
        }

        if stopping {
            // One last drain, so the final tick of audio is not left in the
            // device FIFOs when the streams are dropped below.
            let remainder = drain_remaining(&mic_fifo, &sys_fifo);
            if !remainder.mixed.is_empty() {
                // Blocking, unlike the loop above: this is the last audio of
                // the meeting and the pump is about to be asked to finish, so
                // there is no backlog left to protect against.
                let _ = audio_tx.send(remainder);
            }
            break;
        }

        std::thread::sleep(MIXER_TICK);
    }

    drop(mic_stream);
    drop(sys_stream);
    ctx.mic_active.store(false, Ordering::SeqCst);
    ctx.sys_active.store(false, Ordering::SeqCst);
    emit_levels(&ctx.app, 0.0, 0.0);
}

/// Everything left in both FIFOs, mixed.
fn drain_remaining(
    mic_fifo: &Arc<Mutex<VecDeque<f32>>>,
    sys_fifo: &Arc<Mutex<VecDeque<f32>>>,
) -> MixedAudio {
    let mut mic_guard = mic_fifo.lock_or_recover();
    let mut sys_guard = sys_fifo.lock_or_recover();
    let count = mic_guard.len().max(sys_guard.len());
    let mut mixed = Vec::with_capacity(count);
    let mut mic = Vec::with_capacity(count);
    let mut sys = Vec::with_capacity(count);
    for _ in 0..count {
        let m = mic_guard.pop_front().unwrap_or(0.0);
        let s = sys_guard.pop_front().unwrap_or(0.0);
        mixed.push(soft_mix(m, s));
        mic.push(m);
        sys.push(s);
    }
    MixedAudio {
        mixed,
        mic,
        sys,
        discontinuity: false,
    }
}

/// The microphone to record, honouring a per-recording choice.
///
/// Without a name this is exactly what dictation does, so the two surfaces
/// cannot disagree about which microphone is in use. With one, the named
/// device — and, when that name matches nothing, the shared resolution again,
/// because an unplugged choice should cost the choice and not the recording.
fn resolve_input_device(host: &cpal::Host, name: Option<&str>) -> Option<cpal::Device> {
    if let Some(wanted) = name.map(str::trim).filter(|n| !n.is_empty()) {
        let found = host
            .input_devices()
            .ok()
            .and_then(|mut devices| {
                devices.find(|device| device.name().ok().as_deref() == Some(wanted))
            });
        if found.is_some() {
            return found;
        }
        tracing::info!(
            "meeting capture: microphone '{}' is not connected; falling back",
            wanted
        );
    }
    crate::capture::device::open_preferred(host)
}

/// The output device to capture in loopback — what this machine plays.
///
/// Named separately from the microphone because the far end of a call arrives
/// through whichever output the user is actually listening on, and a laptop
/// with a headset connected has more than one.
fn resolve_output_device(host: &cpal::Host, name: Option<&str>) -> Option<cpal::Device> {
    if let Some(wanted) = name.map(str::trim).filter(|n| !n.is_empty()) {
        let found = host.output_devices().ok().and_then(|mut devices| {
            devices.find(|device| device.name().ok().as_deref() == Some(wanted))
        });
        if found.is_some() {
            return found;
        }
        tracing::info!(
            "meeting capture: output device '{}' is not connected; falling back",
            wanted
        );
    }
    host.default_output_device()
}

/// Binds one capture stream, returning `None` if the device cannot be opened.
///
/// `loopback` selects the system-audio path, where the "input" is the default
/// *output* device captured in loopback mode.
fn build_stream(
    device: Option<cpal::Device>,
    loopback: bool,
    fifo: &Arc<Mutex<VecDeque<f32>>>,
    active_flag: &Arc<AtomicBool>,
    losses: &Arc<AudioLosses>,
) -> Option<cpal::Stream> {
    let label = if loopback { "system audio" } else { "microphone" };
    let Some(device) = device else {
        tracing::warn!("meeting capture: no {} device available", label);
        return None;
    };

    let config = if loopback {
        device
            .default_input_config()
            .or_else(|_| device.default_output_config())
    } else {
        device.default_input_config()
    };
    let config = match config {
        Ok(config) => config,
        Err(err) => {
            tracing::warn!("meeting capture: no usable {} config: {}", label, err);
            return None;
        }
    };

    let sample_rate = config.sample_rate().0;
    let channels = config.channels() as usize;
    let sample_format = config.sample_format();
    let stream_config: cpal::StreamConfig = config.into();

    let err_flag = active_flag.clone();
    let err_label = label.to_string();
    let err_fn = move |err| {
        tracing::error!("meeting capture: {} stream error: {}", err_label, err);
        err_flag.store(false, Ordering::SeqCst);
    };

    macro_rules! build {
        ($sample:ty, $convert:expr) => {{
            let fifo_ref = fifo.clone();
            let losses_ref = Arc::clone(losses);
            device.build_input_stream(
                &stream_config,
                move |data: &[$sample], _: &cpal::InputCallbackInfo| {
                    enqueue_frames(data, channels, sample_rate, $convert, &fifo_ref, &losses_ref);
                },
                err_fn,
                None,
            )
        }};
    }

    let built = match sample_format {
        cpal::SampleFormat::F32 => build!(f32, |s: f32| s),
        cpal::SampleFormat::I16 => build!(i16, |s: i16| s as f32 / i16::MAX as f32),
        cpal::SampleFormat::U16 => build!(u16, |s: u16| (s as f32 - u16::MAX as f32 / 2.0)
            / (u16::MAX as f32 / 2.0)),
        other => {
            tracing::warn!(
                "meeting capture: unsupported {} sample format {:?}",
                label,
                other
            );
            return None;
        }
    };

    let stream = match built {
        Ok(stream) => stream,
        Err(err) => {
            tracing::warn!("meeting capture: could not build {} stream: {}", label, err);
            return None;
        }
    };
    if let Err(err) = stream.play() {
        tracing::warn!("meeting capture: could not start {} stream: {}", label, err);
        return None;
    }

    active_flag.store(true, Ordering::SeqCst);
    tracing::info!(
        "meeting capture: {} active ({}ch @ {}Hz)",
        label,
        channels,
        sample_rate
    );
    Some(stream)
}

/// Audio callback: downmix, resample, enqueue. Nothing else belongs here —
/// metering and event emission happen on the mixer thread so this stays cheap
/// and non-blocking, which is what a real-time audio thread requires.
fn enqueue_frames<T: Copy>(
    data: &[T],
    channels: usize,
    sample_rate: u32,
    convert: impl Fn(T) -> f32,
    fifo: &Arc<Mutex<VecDeque<f32>>>,
    losses: &Arc<AudioLosses>,
) {
    if data.is_empty() || channels == 0 {
        return;
    }
    let mono: Vec<f32> = if channels == 1 {
        data.iter().map(|&s| convert(s)).collect()
    } else {
        data.chunks(channels)
            .map(|frame| frame.iter().map(|&s| convert(s)).sum::<f32>() / frame.len() as f32)
            .collect()
    };
    let resampled = crate::capture::resample_to_16k_mono(&mono, sample_rate);
    let mut guard = fifo.lock_or_recover();
    guard.extend(resampled);
    // The mixer drains this every tick without ever blocking, so reaching the
    // cap means the mixer thread is not running. Discarding the oldest keeps
    // the most recent minute rather than letting the FIFO grow without limit,
    // and counting it is what stops the loss being invisible.
    if guard.len() > MAX_FIFO_SAMPLES {
        let excess = guard.len() - MAX_FIFO_SAMPLES;
        guard.drain(..excess);
        losses
            .fifo_samples_dropped
            .fetch_add(excess as u64, Ordering::SeqCst);
    }
}

/// Decides how many samples to consume from each FIFO this tick.
///
/// See the module docs: lockstep while both streams are delivering, and a
/// bounded catch-up once one has gone silent at the device level.
fn plan_drain(mic_available: usize, sys_available: usize, has_mic: bool, has_sys: bool) -> usize {
    match (has_mic, has_sys) {
        (true, true) => {
            let lockstep = mic_available.min(sys_available);
            if lockstep > 0 {
                lockstep
            } else {
                mic_available
                    .max(sys_available)
                    .saturating_sub(MAX_STREAM_LAG_SAMPLES)
            }
        }
        (true, false) => mic_available,
        (false, true) => sys_available,
        (false, false) => 0,
    }
}

/// Level above which the mix starts compressing instead of summing.
///
/// Below it the mixer is bit-transparent, which is what keeps an ordinary
/// one-speaker meeting untouched.
const MIX_KNEE: f32 = 0.7;

/// Soft-saturating mix of microphone and system audio.
///
/// System audio is attenuated slightly — it is already mastered and typically
/// louder than a voice in a room — and the sum is compressed rather than
/// clipped, because clipping is what turns two people talking at once into a
/// transcript of neither.
///
/// Above [`MIX_KNEE`] the curve is `k + (1-k)(1 - e^-(x-k)/(1-k))`, which is
/// continuous in both value and slope at the knee and approaches 1.0 without
/// reaching it. Vox's previous mixer passed the sum through unchanged up to
/// 1.0 and then evaluated `1 - e^-(x-1)/2`, which is 0.5 just above 1.0: a
/// sample at 1.0 came through at 1.0 and the next one, a hair louder, came
/// through at half that. That discontinuity is a click, and it fired exactly
/// when two people spoke at once — the moment a meeting recorder most needs
/// to stay intelligible.
#[inline]
fn soft_mix(mic: f32, sys: f32) -> f32 {
    let sum = mic + (sys * 0.9);
    let magnitude = sum.abs();
    if magnitude <= MIX_KNEE {
        return sum;
    }
    let headroom = 1.0 - MIX_KNEE;
    let compressed = MIX_KNEE + headroom * (1.0 - (-(magnitude - MIX_KNEE) / headroom).exp());
    compressed.copysign(sum)
}

fn drain_fifo(fifo: &Arc<Mutex<VecDeque<f32>>>) {
    fifo.lock_or_recover().clear();
}

fn mark_heard(flag: &Arc<AtomicBool>, sum_sq: f32, count: usize) {
    if rms_from_sum_sq(sum_sq, count) > AUDIBLE_RMS_THRESHOLD {
        flag.store(true, Ordering::SeqCst);
    }
}

fn rms_from_sum_sq(sum_sq: f32, count: usize) -> f32 {
    if count == 0 {
        return 0.0;
    }
    (sum_sq / count as f32).sqrt()
}

/// Maps an RMS reading onto `0.0..=1.0` for the recording surface's meters.
///
/// Display only — every audibility decision uses RMS directly, so this curve
/// can be tuned for the eye without changing what the recorder considers
/// audible. A meter is perceptual: scaling RMS linearly puts speech at 0.1–0.4
/// of full scale, which reads as a flat line.
fn meter_level(rms: f32) -> f32 {
    if rms <= AUDIBLE_RMS_THRESHOLD {
        return 0.0;
    }
    let db = 20.0 * rms.log10();
    ((db - METER_FLOOR_DB) / (METER_CEILING_DB - METER_FLOOR_DB)).clamp(0.0, 1.0)
}

fn emit_levels(app: &Option<AppHandle>, mic: f32, system: f32) {
    if let Some(handle) = app {
        let _ = handle.emit(MEETING_LEVEL_EVENT, MeetingLevels { mic, system });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_streams_live_are_consumed_in_exact_lockstep() {
        assert_eq!(plan_drain(500, 300, true, true), 300);
        assert_eq!(plan_drain(300, 500, true, true), 300);
    }

    #[test]
    fn a_silent_loopback_does_not_stall_the_recording() {
        // WASAPI loopback delivers nothing while no audio plays. Once the
        // microphone is far enough ahead the mixer must advance anyway.
        let drained = plan_drain(MAX_STREAM_LAG_SAMPLES + 1_000, 0, true, true);
        assert_eq!(drained, 1_000);
        // Inside the lag window it still waits, so ordinary jitter does not
        // insert padding.
        assert_eq!(plan_drain(MAX_STREAM_LAG_SAMPLES - 1, 0, true, true), 0);
    }

    #[test]
    fn a_single_bound_stream_is_drained_whole() {
        assert_eq!(plan_drain(700, 0, true, false), 700);
        assert_eq!(plan_drain(0, 700, false, true), 700);
        assert_eq!(plan_drain(700, 700, false, false), 0);
    }

    #[test]
    fn mixing_two_loud_sources_compresses_instead_of_clipping() {
        let mixed = soft_mix(0.9, 0.9);
        assert!(mixed <= 1.0, "mix must stay in range, got {mixed}");
        assert!(mixed > MIX_KNEE, "compression must not duck below the knee");
        assert!(soft_mix(-0.9, -0.9) >= -1.0);
        assert!((soft_mix(-0.9, -0.9) + soft_mix(0.9, 0.9)).abs() < 1e-6, "symmetric");
    }

    #[test]
    fn mixing_is_transparent_at_ordinary_levels() {
        // Nothing should be reshaped until the sum reaches the knee.
        assert!((soft_mix(0.2, 0.0) - 0.2).abs() < 1e-6);
        assert!((soft_mix(0.3, 0.3) - 0.57).abs() < 1e-6);
    }

    #[test]
    fn the_mix_curve_has_no_step_in_it() {
        // The previous mixer dropped from 1.0 to 0.5 across one sample's worth
        // of level, which is an audible click at exactly the moment two people
        // talk over each other.
        let mut previous = soft_mix(0.0, 0.0);
        let mut x = 0.0f32;
        while x <= 2.0 {
            let value = soft_mix(x, 0.0);
            assert!(
                value >= previous - 1e-6,
                "mix curve went backwards at {x}: {previous} -> {value}"
            );
            assert!(
                value - previous < 0.02,
                "mix curve stepped at {x}: {previous} -> {value}"
            );
            assert!(value <= 1.0, "mix exceeded full scale at {x}");
            previous = value;
            x += 0.01;
        }
    }

    #[test]
    fn the_meter_reads_flat_below_the_silence_floor_and_rises_with_level() {
        assert_eq!(meter_level(0.0), 0.0);
        assert_eq!(meter_level(AUDIBLE_RMS_THRESHOLD), 0.0);
        let quiet = meter_level(0.01);
        let speech = meter_level(0.05);
        let loud = meter_level(0.3);
        assert!(quiet < speech && speech < loud);
        assert!(loud <= 1.0);
        // Conversational speech should land in the visible middle of the bar,
        // not pinned against either end.
        assert!((0.25..=0.85).contains(&speech), "speech read {speech}");
    }

    #[test]
    fn a_source_that_never_exceeds_the_floor_is_not_marked_heard() {
        let flag = Arc::new(AtomicBool::new(false));
        mark_heard(&flag, 0.000_001 * 100.0, 100);
        assert!(!flag.load(Ordering::SeqCst));
        mark_heard(&flag, 0.05 * 0.05 * 100.0, 100);
        assert!(flag.load(Ordering::SeqCst));
    }

    #[test]
    fn rms_of_nothing_is_zero_rather_than_a_division_by_zero() {
        assert_eq!(rms_from_sum_sq(0.0, 0), 0.0);
    }
    #[test]
    fn a_fifo_the_mixer_stopped_draining_keeps_the_newest_minute_and_counts_the_rest() {
        // The mixer drains every tick and never blocks, so reaching the cap
        // means the mixer thread is not running. At that point the choice is
        // between a bounded loss and an unbounded one — and the loss has to be
        // counted, or the recording quietly has a hole in it.
        let fifo: Arc<Mutex<VecDeque<f32>>> = Arc::new(Mutex::new(VecDeque::new()));
        let losses = Arc::new(AudioLosses::default());

        let overflow = MAX_FIFO_SAMPLES + 8_000;
        // Real sample values: the resampler clamps to full scale, so a plain
        // index ramp would arrive as a wall of 1.0 and prove nothing.
        let mut data = vec![0.25f32; overflow];
        *data.last_mut().unwrap() = -0.5;
        enqueue_frames(&data, 1, SEGMENT_SAMPLE_RATE, |s: f32| s, &fifo, &losses);

        let guard = fifo.lock_or_recover();
        assert_eq!(guard.len(), MAX_FIFO_SAMPLES, "the cap must hold");
        assert_eq!(
            losses.fifo_samples_dropped.load(Ordering::SeqCst),
            8_000,
            "and every discarded sample must be counted"
        );
        // The newest audio survives, not the oldest: what someone just said
        // matters more than what they said a minute ago.
        assert_eq!(*guard.back().unwrap(), -0.5);
    }

    #[test]
    fn a_fifo_inside_the_cap_is_left_alone() {
        let fifo: Arc<Mutex<VecDeque<f32>>> = Arc::new(Mutex::new(VecDeque::new()));
        let losses = Arc::new(AudioLosses::default());
        enqueue_frames(&vec![0.1f32; 16_000], 1, SEGMENT_SAMPLE_RATE, |s| s, &fifo, &losses);
        assert_eq!(fifo.lock_or_recover().len(), 16_000);
        assert!(!losses.any());
    }

    #[test]
    fn lost_audio_is_reported_in_seconds_across_both_causes() {
        let losses = AudioLosses::default();
        assert!(!losses.any());
        assert_eq!(losses.lost_seconds(320), 0.0);

        // Fifty shed blocks of 20 ms, plus half a second of FIFO overrun.
        losses.blocks_shed.store(50, Ordering::SeqCst);
        losses
            .fifo_samples_dropped
            .store(SEGMENT_SAMPLE_RATE as u64 / 2, Ordering::SeqCst);
        assert!(losses.any());
        assert!((losses.lost_seconds(320) - 1.5).abs() < 1e-9);
    }

    #[test]
    fn the_channel_to_the_pump_is_bounded_at_ten_seconds_of_audio() {
        // The bound is the whole point: unbounded, a stalled pump grows memory
        // until the process dies, which costs every second since the last
        // checkpoint rather than the twenty milliseconds shedding costs.
        let blocks_per_second = 1000 / MIXER_TICK.as_millis() as usize;
        assert_eq!(AUDIO_CHANNEL_BLOCKS / blocks_per_second, 10);
    }

    #[test]
    fn device_selection_is_empty_until_something_is_chosen() {
        assert!(MeetingDevices::default().is_empty());
        assert!(!MeetingDevices {
            microphone: Some("Yeti".into()),
            ..Default::default()
        }
        .is_empty());
        assert!(!MeetingDevices {
            system_audio: Some("Speakers".into()),
            ..Default::default()
        }
        .is_empty());
    }

    #[test]
    fn device_selection_round_trips_through_settings_json() {
        // It is stored in `AppSettings`, so an older settings file with no
        // `devices` key has to load rather than fail the whole document.
        let parsed: MeetingDevices = serde_json::from_str("{}").unwrap();
        assert!(parsed.is_empty());

        let chosen = MeetingDevices {
            microphone: Some("Yeti Stereo Microphone".into()),
            system_audio: Some("Speakers (Realtek)".into()),
        };
        let round_tripped: MeetingDevices =
            serde_json::from_str(&serde_json::to_string(&chosen).unwrap()).unwrap();
        assert_eq!(round_tripped, chosen);
    }
}
