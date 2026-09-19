//! Where meetings live on disk, and how they survive a crash.
//!
//! One directory per meeting under `<vault>/meetings/`, the same shape the
//! vault already uses for imported files and web captures:
//!
//! ```text
//! <vault>/meetings/<meeting_id>/
//! ├── meeting.json      metadata — the record the list surface reads
//! ├── transcript.json   Vec<TranscriptSegment>, ordered by sequence
//! ├── summary.json      the generated report and its cache fingerprint
//! ├── notes.md          whatever the user typed during the meeting
//! └── audio/
//!     ├── chunk_000000.wav   durable 30s checkpoints, during recording
//!     └── audio.wav          the merged recording, written on stop
//! ```
//!
//! Files rather than a database, because that is what the rest of this vault
//! is: a meeting stays greppable, syncable and readable without Vox running.
//! Meetily reaches for SQLite here and then leaves
//! `transcripts.meeting_id` — the column every one of its queries filters and
//! orders on — unindexed, so the database buys it schema without buying it
//! speed.
//!
//! Every write that matters goes to a temporary file and is renamed into
//! place. A half-written `transcript.json` is worse than a stale one.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use thiserror::Error;

use crate::sync::MutexExt;

use super::series::MeetingSeries;
use super::model::{
    Meeting, MeetingListItem, MeetingState, MeetingSummary, SummaryStatus, TranscriptSegment,
};
use super::telemetry::{MeetingDiagnostics, SegmentDiagnostics};

/// Directory under the vault root that holds every meeting.
pub const MEETINGS_DIR: &str = "meetings";

const MEETING_FILE: &str = "meeting.json";
const TRANSCRIPT_FILE: &str = "transcript.json";
/// The live transcript, kept when a final pass replaces it.
const LIVE_TRANSCRIPT_FILE: &str = "transcript.live.json";
const SUMMARY_FILE: &str = "summary.json";
const NOTES_FILE: &str = "notes.md";
const SPEAKERS_FILE: &str = "speakers.json";
/// Which transcript line belongs to whom. Separate from the transcript
/// because attribution is an interpretation of evidence, not evidence.
const ATTRIBUTION_FILE: &str = "attribution.json";
/// Per-segment decode telemetry, one JSON object per line.
const SEGMENT_DIAGNOSTICS_FILE: &str = "diagnostics.jsonl";
/// The meeting's diagnostics rollup, written once on stop.
const DIAGNOSTICS_FILE: &str = "diagnostics.json";
const AUDIO_DIR: &str = "audio";
/// Where the recurring-meeting records live, beside the meetings themselves.
const SERIES_FILE: &str = "series.json";

/// Transcripts held in the parse cache at once.
///
/// Small on purpose. The cache exists so the meeting being recorded does not
/// re-parse its own file on every append — one transcript, plus a little room
/// for a detail view open beside it. Anything larger is a vault browsed, not a
/// meeting recorded, and holding every transcript opened since launch is a
/// leak wearing a cache's clothes.
const MAX_CACHED_TRANSCRIPTS: usize = 4;

/// Characters in the preview shown on a meeting list row.
const PREVIEW_CHARS: usize = 200;

#[derive(Debug, Error)]
pub enum MeetingStoreError {
    #[error("meeting storage IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("meeting record could not be read or written: {0}")]
    Serde(String),

    #[error("no meeting with id {0}")]
    NotFound(String),

    #[error("meeting id {0} is not a valid identifier")]
    InvalidId(String),
}

impl From<serde_json::Error> for MeetingStoreError {
    fn from(err: serde_json::Error) -> Self {
        MeetingStoreError::Serde(err.to_string())
    }
}

/// Reads and writes meetings under a vault root.
///
/// The root is behind a `Mutex` for the same reason [`crate::vault::VaultManager`]'s
/// is: the user can repoint their vault at runtime, and every method here only
/// ever needed `&self`.
pub struct MeetingStore {
    vault_dir: Mutex<PathBuf>,
    /// The parsed transcript of whichever meetings have been touched, and the
    /// modification time the parse was of.
    ///
    /// `append_segments` rewrites the whole file — the right trade for a
    /// document that has to stay atomically replaceable — and it used to
    /// re-read and re-parse it first, every time, on the decode thread.
    /// Measured over a synthetic two-hour meeting: the first hundred lines
    /// cost 40 ms to persist and the last hundred cost 2056 ms, fifty-one
    /// times more, because each append paid for everything already written.
    /// See `meetings::endurance`.
    ///
    /// Keyed by meeting id and validated against the file's modification
    /// time, so a transcript edited outside Vox — which the vault is
    /// explicitly meant to allow — is re-read rather than served stale.
    transcripts: Mutex<HashMap<String, CachedTranscript>>,
}

/// A parsed transcript, and the file state it was parsed from.
struct CachedTranscript {
    modified: Option<std::time::SystemTime>,
    segments: Vec<TranscriptSegment>,
}

impl MeetingStore {
    pub fn new(vault_dir: impl Into<PathBuf>) -> Self {
        Self {
            vault_dir: Mutex::new(vault_dir.into()),
            transcripts: Mutex::new(HashMap::new()),
        }
    }

    /// Repoints the store at a new vault root. Meetings at the old location
    /// stay where they are, matching how the rest of the vault behaves.
    pub fn set_vault_dir(&self, new_dir: impl Into<PathBuf>) {
        *self.vault_dir.lock_or_recover() = new_dir.into();
        // Meeting ids are unique, but a new vault is a different set of
        // meetings and nothing cached under the old root describes it.
        self.transcripts.lock_or_recover().clear();
    }

    /// The `meetings/` directory under the current vault root.
    pub fn meetings_dir(&self) -> PathBuf {
        self.vault_dir.lock_or_recover().join(MEETINGS_DIR)
    }

    /// One meeting's directory. Rejects an id that could escape the vault.
    pub fn meeting_dir(&self, id: &str) -> Result<PathBuf, MeetingStoreError> {
        if !is_safe_id(id) {
            return Err(MeetingStoreError::InvalidId(id.to_string()));
        }
        Ok(self.meetings_dir().join(id))
    }

    /// Where a meeting's audio chunks and merged recording live.
    pub fn audio_dir(&self, id: &str) -> Result<PathBuf, MeetingStoreError> {
        Ok(self.meeting_dir(id)?.join(AUDIO_DIR))
    }

    /// Creates the directory tree for a new meeting and writes its record.
    pub fn create(&self, meeting: &Meeting) -> Result<PathBuf, MeetingStoreError> {
        let dir = self.meeting_dir(&meeting.id)?;
        fs::create_dir_all(dir.join(AUDIO_DIR))?;
        self.save_meeting(meeting)?;
        self.save_transcript(&meeting.id, &[])?;
        Ok(dir)
    }

    /// Writes a meeting record, stamping `updated_at`.
    pub fn save_meeting(&self, meeting: &Meeting) -> Result<(), MeetingStoreError> {
        let dir = self.meeting_dir(&meeting.id)?;
        fs::create_dir_all(&dir)?;
        let mut record = meeting.clone();
        record.touch();
        write_atomic(&dir.join(MEETING_FILE), &serde_json::to_vec_pretty(&record)?)
    }

    pub fn load_meeting(&self, id: &str) -> Result<Meeting, MeetingStoreError> {
        let path = self.meeting_dir(id)?.join(MEETING_FILE);
        if !path.exists() {
            return Err(MeetingStoreError::NotFound(id.to_string()));
        }
        Ok(serde_json::from_slice(&fs::read(&path)?)?)
    }

    /// Applies `edit` to a stored meeting and writes it back.
    ///
    /// The one mutation path, so "read, change one field, write" cannot
    /// accidentally drop a field another thread just set.
    pub fn update_meeting<F>(&self, id: &str, edit: F) -> Result<Meeting, MeetingStoreError>
    where
        F: FnOnce(&mut Meeting),
    {
        let mut meeting = self.load_meeting(id)?;
        edit(&mut meeting);
        self.save_meeting(&meeting)?;
        self.load_meeting(id)
    }

    /// Every meeting, newest first.
    ///
    /// A directory that fails to parse is skipped with a warning rather than
    /// failing the whole listing — one corrupt meeting must not hide the
    /// other fifty.
    pub fn list_meetings(&self) -> Result<Vec<Meeting>, MeetingStoreError> {
        let dir = self.meetings_dir();
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut meetings = Vec::new();
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let record = entry.path().join(MEETING_FILE);
            if !record.exists() {
                continue;
            }
            match fs::read(&record).map_err(MeetingStoreError::from).and_then(|bytes| {
                serde_json::from_slice::<Meeting>(&bytes).map_err(MeetingStoreError::from)
            }) {
                Ok(meeting) => meetings.push(meeting),
                Err(err) => {
                    tracing::warn!("skipping unreadable meeting at {:?}: {}", record, err);
                }
            }
        }
        meetings.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(meetings)
    }

    /// The list surface's view: metadata plus whether a summary exists and a
    /// short transcript preview, so the UI never fetches a full transcript to
    /// render a row.
    pub fn list_for_display(&self) -> Result<Vec<MeetingListItem>, MeetingStoreError> {
        let meetings = self.list_meetings()?;
        let mut items = Vec::with_capacity(meetings.len());
        for meeting in meetings {
            let summary = self.load_summary(&meeting.id).ok().flatten();
            let preview = self
                .load_transcript(&meeting.id)
                .map(|segments| transcript_preview(&segments, PREVIEW_CHARS))
                .unwrap_or_default();
            items.push(MeetingListItem {
                has_summary: summary
                    .as_ref()
                    .is_some_and(|s| s.status == SummaryStatus::Completed),
                summary_status: summary.as_ref().map(|s| s.status),
                preview,
                meeting,
            });
        }
        Ok(items)
    }

    pub fn save_transcript(
        &self,
        id: &str,
        segments: &[TranscriptSegment],
    ) -> Result<(), MeetingStoreError> {
        let dir = self.meeting_dir(id)?;
        fs::create_dir_all(&dir)?;
        let path = dir.join(TRANSCRIPT_FILE);
        write_atomic(&path, &serde_json::to_vec_pretty(segments)?)?;
        self.remember_transcript(id, modified_time(&path), segments.to_vec());
        Ok(())
    }

    /// Puts a parsed transcript in the cache, within its bound.
    ///
    /// One place, because there are two ways in — a write and a read — and a
    /// bound applied to only one of them is not a bound. Past the cap the
    /// cheapest correct thing is to start again: tracking recency would mean
    /// an ordering the rest of the store has no use for, to choose between
    /// entries that are all cheap to rebuild.
    fn remember_transcript(
        &self,
        id: &str,
        modified: Option<std::time::SystemTime>,
        segments: Vec<TranscriptSegment>,
    ) {
        let mut cache = self.transcripts.lock_or_recover();
        if cache.len() >= MAX_CACHED_TRANSCRIPTS && !cache.contains_key(id) {
            cache.clear();
        }
        cache.insert(id.to_string(), CachedTranscript { modified, segments });
    }

    /// A meeting's transcript, ordered by sequence.
    ///
    /// Sorting on read rather than trusting the file means a transcript
    /// written by an interrupted append is still in the right order.
    pub fn load_transcript(&self, id: &str) -> Result<Vec<TranscriptSegment>, MeetingStoreError> {
        let path = self.meeting_dir(id)?.join(TRANSCRIPT_FILE);
        if !path.exists() {
            self.transcripts.lock_or_recover().remove(id);
            return Ok(Vec::new());
        }
        let modified = modified_time(&path);
        if modified.is_some() {
            let cache = self.transcripts.lock_or_recover();
            if let Some(cached) = cache.get(id) {
                // An unchanged file is the common case by a wide margin, and
                // parsing it again is most of what made a long meeting's
                // thousandth line expensive to write.
                if cached.modified == modified {
                    return Ok(cached.segments.clone());
                }
            }
        }

        let mut segments: Vec<TranscriptSegment> = serde_json::from_slice(&fs::read(&path)?)?;
        segments.sort_by_key(|segment| segment.sequence);
        segments.dedup_by_key(|segment| segment.sequence);
        self.remember_transcript(id, modified, segments.clone());
        Ok(segments)
    }

    /// Adds segments to a stored transcript, keeping it sequence-ordered and
    /// free of duplicates.
    ///
    /// Rewrites the whole file. That is the right trade at meeting scale — a
    /// three-hour meeting is a few hundred kilobytes of JSON — and it is what
    /// keeps the file atomically replaceable.
    pub fn append_segments(
        &self,
        id: &str,
        new_segments: &[TranscriptSegment],
    ) -> Result<usize, MeetingStoreError> {
        if new_segments.is_empty() {
            return Ok(self.load_transcript(id)?.len());
        }
        let mut segments = self.load_transcript(id)?;
        segments.extend_from_slice(new_segments);
        segments.sort_by_key(|segment| segment.sequence);
        segments.dedup_by_key(|segment| segment.sequence);
        let total = segments.len();
        self.save_transcript(id, &segments)?;
        Ok(total)
    }

    /// Appends one segment's telemetry to the meeting's diagnostics log.
    ///
    /// Appended a line at a time rather than rewritten, unlike every other
    /// document here. Two reasons, and they both point the same way: this is a
    /// log rather than a document, so recording the thousandth segment must
    /// cost what the first did; and a crash should keep every line written
    /// before it rather than losing the file. A torn final line is tolerable
    /// because [`Self::load_segment_diagnostics`] skips what it cannot parse —
    /// for a transcript that would be unacceptable, and for telemetry about a
    /// run that has already crashed it is exactly right.
    pub fn append_segment_diagnostics(
        &self,
        id: &str,
        record: &SegmentDiagnostics,
    ) -> Result<(), MeetingStoreError> {
        use std::io::Write;

        let dir = self.meeting_dir(id)?;
        fs::create_dir_all(&dir)?;
        let mut line = serde_json::to_vec(record)?;
        line.push(b'\n');
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join(SEGMENT_DIAGNOSTICS_FILE))?;
        file.write_all(&line)?;
        Ok(())
    }

    /// Every segment record for a meeting, in sequence order.
    ///
    /// A line that will not parse is skipped rather than failing the read: the
    /// common reason for one is the crash whose diagnostics are being read.
    pub fn load_segment_diagnostics(
        &self,
        id: &str,
    ) -> Result<Vec<SegmentDiagnostics>, MeetingStoreError> {
        let path = self.meeting_dir(id)?.join(SEGMENT_DIAGNOSTICS_FILE);
        if !path.exists() {
            return Ok(Vec::new());
        }
        let raw = fs::read_to_string(&path)?;
        let mut records: Vec<SegmentDiagnostics> = raw
            .lines()
            .filter(|line| !line.trim().is_empty())
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();
        records.sort_by_key(|record| record.sequence);
        Ok(records)
    }

    pub fn save_diagnostics(
        &self,
        diagnostics: &MeetingDiagnostics,
    ) -> Result<(), MeetingStoreError> {
        let dir = self.meeting_dir(&diagnostics.meeting_id)?;
        fs::create_dir_all(&dir)?;
        write_atomic(
            &dir.join(DIAGNOSTICS_FILE),
            &serde_json::to_vec_pretty(diagnostics)?,
        )
    }

    pub fn load_diagnostics(
        &self,
        id: &str,
    ) -> Result<Option<MeetingDiagnostics>, MeetingStoreError> {
        let path = self.meeting_dir(id)?.join(DIAGNOSTICS_FILE);
        if !path.exists() {
            return Ok(None);
        }
        Ok(serde_json::from_slice(&fs::read(&path)?).ok())
    }

    /// Copies the live transcript aside before a final pass replaces it.
    ///
    /// Written once and never overwritten: the point is to keep what the
    /// *live* pass produced, and a second final pass would otherwise archive
    /// the first final pass over it and lose the thing worth comparing
    /// against.
    ///
    /// Two jobs at once. A crash part-way through a re-transcription used to
    /// leave a truncated `transcript.json` and nothing to restore from, since
    /// the only copy was in the caller's memory. And "the new transcript is
    /// worse than the old one" was unanswerable without one to compare.
    ///
    /// Returns whether an archive was made.
    pub fn archive_live_transcript(&self, id: &str) -> Result<bool, MeetingStoreError> {
        let dir = self.meeting_dir(id)?;
        let archive = dir.join(LIVE_TRANSCRIPT_FILE);
        if archive.exists() {
            return Ok(false);
        }
        // An *empty* transcript must not be archived. `create` writes one, so
        // a meeting that produced no text at all would otherwise fill the
        // once-only archive slot with nothing and hide the real live
        // transcript from every later pass.
        let current = self.load_transcript(id)?;
        if current.is_empty() {
            return Ok(false);
        }
        write_atomic(&archive, &serde_json::to_vec_pretty(&current)?)?;
        Ok(true)
    }

    /// The live transcript, where a final pass has since replaced it.
    ///
    /// `None` means no final pass has run, so `transcript.json` *is* the live
    /// one — not that the live transcript was lost.
    pub fn load_live_transcript(
        &self,
        id: &str,
    ) -> Result<Option<Vec<TranscriptSegment>>, MeetingStoreError> {
        let path = self.meeting_dir(id)?.join(LIVE_TRANSCRIPT_FILE);
        if !path.exists() {
            return Ok(None);
        }
        let mut segments: Vec<TranscriptSegment> = serde_json::from_slice(&fs::read(&path)?)?;
        segments.sort_by_key(|segment| segment.sequence);
        Ok(Some(segments))
    }

    pub fn save_attribution(
        &self,
        id: &str,
        attribution: &super::speakers::SpeakerAttribution,
    ) -> Result<(), MeetingStoreError> {
        let dir = self.meeting_dir(id)?;
        fs::create_dir_all(&dir)?;
        write_atomic(
            &dir.join(ATTRIBUTION_FILE),
            &serde_json::to_vec_pretty(attribution)?,
        )
    }

    /// Who said what, as far as detection has got.
    ///
    /// An empty attribution for a meeting where detection has not run, which
    /// is not an error: a transcript labelled "You" and "Others" from the
    /// capture channel is still a transcript, and the channel is measured
    /// rather than inferred.
    pub fn load_attribution(
        &self,
        id: &str,
    ) -> Result<super::speakers::SpeakerAttribution, MeetingStoreError> {
        let path = self.meeting_dir(id)?.join(ATTRIBUTION_FILE);
        if !path.exists() {
            return Ok(super::speakers::SpeakerAttribution::default());
        }
        Ok(serde_json::from_slice(&fs::read(&path)?).unwrap_or_default())
    }

    pub fn save_summary(&self, summary: &MeetingSummary) -> Result<(), MeetingStoreError> {
        let dir = self.meeting_dir(&summary.meeting_id)?;
        fs::create_dir_all(&dir)?;
        write_atomic(&dir.join(SUMMARY_FILE), &serde_json::to_vec_pretty(summary)?)
    }

    pub fn load_summary(&self, id: &str) -> Result<Option<MeetingSummary>, MeetingStoreError> {
        let path = self.meeting_dir(id)?.join(SUMMARY_FILE);
        if !path.exists() {
            return Ok(None);
        }
        Ok(Some(serde_json::from_slice(&fs::read(&path)?)?))
    }

    /// Free-text notes the user typed against a meeting.
    /// The speakers a detection run proposed, and the names the user gave them.
    ///
    /// Its own file rather than a field on `meeting.json`: a detection run
    /// rewrites this wholesale and must not be able to lose the meeting's
    /// metadata by failing halfway.
    pub fn save_speakers(
        &self,
        id: &str,
        speakers: &[crate::meetings::model::Speaker],
    ) -> Result<(), MeetingStoreError> {
        let dir = self.meeting_dir(id)?;
        fs::create_dir_all(&dir)?;
        write_atomic(
            &dir.join(SPEAKERS_FILE),
            &serde_json::to_vec_pretty(speakers)?,
        )
    }

    /// A meeting's speakers, or none where detection has never run.
    ///
    /// A file that cannot be parsed reads as "no speakers" rather than as an
    /// error: the transcript is the product, and a meeting must still open
    /// when a detection run left a half-written file behind.
    pub fn load_speakers(
        &self,
        id: &str,
    ) -> Result<Vec<crate::meetings::model::Speaker>, MeetingStoreError> {
        let path = self.meeting_dir(id)?.join(SPEAKERS_FILE);
        if !path.exists() {
            return Ok(Vec::new());
        }
        Ok(serde_json::from_slice(&fs::read(&path)?).unwrap_or_default())
    }

    pub fn load_notes(&self, id: &str) -> Result<String, MeetingStoreError> {
        let path = self.meeting_dir(id)?.join(NOTES_FILE);
        if !path.exists() {
            return Ok(String::new());
        }
        Ok(fs::read_to_string(&path)?)
    }

    pub fn save_notes(&self, id: &str, notes: &str) -> Result<(), MeetingStoreError> {
        let dir = self.meeting_dir(id)?;
        fs::create_dir_all(&dir)?;
        write_atomic(&dir.join(NOTES_FILE), notes.as_bytes())
    }

    // --- recurring meeting series ---------------------------------------

    /// Every recurring meeting Vox knows about.
    ///
    /// A file that cannot be read means no series rather than an error, the
    /// way the calendar's own cache behaves: a corrupt index must not make the
    /// meetings underneath it unopenable.
    pub fn load_series(&self) -> Vec<MeetingSeries> {
        let path = self.meetings_dir().join(SERIES_FILE);
        if !path.exists() {
            return Vec::new();
        }
        fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save_series(&self, series: &[MeetingSeries]) -> Result<(), MeetingStoreError> {
        let dir = self.meetings_dir();
        fs::create_dir_all(&dir)?;
        write_atomic(&dir.join(SERIES_FILE), &serde_json::to_vec_pretty(series)?)
    }

    /// Adds a series, or refreshes the title of one already known.
    ///
    /// A title the user has set is left alone: the calendar renaming a
    /// recurrence must not undo the name somebody chose for it here.
    pub fn upsert_series(&self, entry: MeetingSeries) -> Result<Vec<MeetingSeries>, MeetingStoreError> {
        let mut all = self.load_series();
        match all.iter_mut().find(|existing| existing.id == entry.id) {
            Some(existing) => {
                if !existing.renamed_by_user && !entry.title.trim().is_empty() {
                    existing.title = entry.title;
                }
            }
            None => all.push(entry),
        }
        self.save_series(&all)?;
        Ok(all)
    }

    /// Renames a series, and records that the name is the user's from now on.
    pub fn rename_series(&self, id: &str, title: &str) -> Result<Vec<MeetingSeries>, MeetingStoreError> {
        let mut all = self.load_series();
        let Some(entry) = all.iter_mut().find(|existing| existing.id == id) else {
            return Err(MeetingStoreError::NotFound(id.to_string()));
        };
        entry.title = title.trim().to_string();
        entry.renamed_by_user = true;
        self.save_series(&all)?;
        Ok(all)
    }

    /// Forgets a series. Its meetings stay; they simply stop being in one.
    pub fn delete_series(&self, id: &str) -> Result<(), MeetingStoreError> {
        let mut all = self.load_series();
        all.retain(|existing| existing.id != id);
        self.save_series(&all)?;

        for meeting in self.list_meetings().unwrap_or_default() {
            if meeting.series_id.as_deref() == Some(id) {
                let _ = self.update_meeting(&meeting.id, |record| record.series_id = None);
            }
        }
        Ok(())
    }

    /// Puts one meeting in a series, or takes it out of the one it is in.
    pub fn set_meeting_series(
        &self,
        meeting_id: &str,
        series_id: Option<String>,
    ) -> Result<Meeting, MeetingStoreError> {
        self.update_meeting(meeting_id, |record| record.series_id = series_id.clone())
    }

    /// Deletes a meeting's entire directory, audio included.
    ///
    /// Meetily deletes the database rows and leaves `audio.mp4` on disk
    /// forever, so a user who deletes a sensitive meeting has not deleted the
    /// recording. Here the directory is the meeting; removing it removes all
    /// of it.
    pub fn delete_meeting(&self, id: &str) -> Result<(), MeetingStoreError> {
        self.transcripts.lock_or_recover().remove(id);
        let dir = self.meeting_dir(id)?;
        if !dir.exists() {
            return Err(MeetingStoreError::NotFound(id.to_string()));
        }
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    /// Case-insensitive substring search across every transcript, returning
    /// each hit with surrounding context.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<MeetingSearchHit>, MeetingStoreError> {
        let needle = query.trim().to_lowercase();
        if needle.is_empty() {
            return Ok(Vec::new());
        }
        let mut hits = Vec::new();
        for meeting in self.list_meetings()? {
            let segments = self.load_transcript(&meeting.id).unwrap_or_default();
            for segment in &segments {
                if segment.text.to_lowercase().contains(&needle) {
                    hits.push(MeetingSearchHit {
                        meeting_id: meeting.id.clone(),
                        meeting_title: meeting.title.clone(),
                        sequence: segment.sequence,
                        start_seconds: segment.start_seconds,
                        excerpt: excerpt_around(&segment.text, &needle, 120),
                    });
                    if hits.len() >= limit {
                        return Ok(hits);
                    }
                }
            }
        }
        Ok(hits)
    }

    /// Finalizes meetings a crash left mid-recording.
    ///
    /// A meeting still marked `Recording` or `Paused` at launch cannot be
    /// resumed — its capture threads and its in-memory queue died with the
    /// process. What survives is its durable audio chunks and whatever
    /// transcript was flushed. This walks those, recomputes the true duration
    /// from the audio on disk, and marks the meeting `Completed` so it shows
    /// up as the (partial) record it is rather than as a recording that never
    /// ends.
    ///
    /// Returns the ids it recovered.
    pub fn recover_interrupted(&self) -> Result<Vec<String>, MeetingStoreError> {
        let mut recovered = Vec::new();
        for meeting in self.list_meetings()? {
            if let Ok(Some(mut summary)) = self.load_summary(&meeting.id) {
                if summary.is_running() {
                    summary.status = SummaryStatus::Failed;
                    summary.error = Some(
                        "Generation was interrupted when Vox closed or restarted."
                            .to_string(),
                    );
                    summary.markdown = summary.previous_markdown.take().or(summary.markdown);
                    summary.completed_at = Some(chrono::Utc::now().to_rfc3339());
                    let _ = self.save_summary(&summary);
                }
            }

            if !meeting.state.is_interrupted() {
                continue;
            }
            let audio_dir = self.audio_dir(&meeting.id)?;
            let chunks = list_chunks(&audio_dir);
            let duration = chunks
                .iter()
                .filter_map(|path| wav_duration_seconds(path))
                .sum::<f64>();
            let segments = self.load_transcript(&meeting.id).unwrap_or_default();

            let id = meeting.id.clone();
            self.update_meeting(&id, |record| {
                record.state = MeetingState::Completed;
                // A duration measured from the audio that actually reached
                // disk, not the clock the dead process was keeping.
                if duration > 0.0 {
                    record.duration_seconds = duration;
                }
                record.segment_count = segments.len();
                record.error = Some(
                    "Recovered after Vox closed unexpectedly. Audio and transcript are complete \
                     up to the last saved checkpoint."
                        .to_string(),
                );
            })?;
            tracing::info!(
                "recovered interrupted meeting {} ({} chunks, {:.1}s)",
                id,
                chunks.len(),
                duration
            );
            recovered.push(id);
        }
        Ok(recovered)
    }
}

/// One transcript match, with enough context to render a result row.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MeetingSearchHit {
    pub meeting_id: String,
    pub meeting_title: String,
    pub sequence: u64,
    pub start_seconds: f64,
    pub excerpt: String,
}

/// Whether an id is safe to join onto the vault path.
///
/// Ids are generated, so this only ever rejects a caller that made one up —
/// but a command boundary takes ids from the frontend, and meetily's template
/// loader shows what the missing check costs: `template_id` is joined into a
/// path with no traversal guard (`templates/loader.rs:41`), so `../../…`
/// reaches any file the user can read.
fn is_safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// A file's modification time, or `None` where the platform will not say.
///
/// `None` disables the transcript cache for that file rather than assuming it
/// is unchanged: a cache that cannot tell whether it is stale must not claim
/// it is fresh.
fn modified_time(path: &Path) -> Option<std::time::SystemTime> {
    fs::metadata(path).and_then(|meta| meta.modified()).ok()
}

/// Writes bytes to `path` through a temporary file in the same directory.
///
/// Same-directory, because a rename across filesystems is a copy and stops
/// being atomic.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), MeetingStoreError> {
    let parent = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let tmp = path.with_extension(format!(
        "tmp{}",
        std::process::id()
    ));
    fs::write(&tmp, bytes)?;
    match fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(err) => {
            let _ = fs::remove_file(&tmp);
            Err(err.into())
        }
    }
}

/// A meeting's durable audio checkpoints, in capture order.
pub fn list_chunks(audio_dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(audio_dir) else {
        return Vec::new();
    };
    let mut chunks: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("chunk_") && name.ends_with(".wav"))
        })
        .collect();
    chunks.sort();
    chunks
}

/// How long a 16-bit PCM WAV runs, read from its header rather than decoded.
fn wav_duration_seconds(path: &Path) -> Option<f64> {
    let reader = hound::WavReader::open(path).ok()?;
    let spec = reader.spec();
    let frames = reader.len() as f64 / spec.channels.max(1) as f64;
    Some(frames / spec.sample_rate.max(1) as f64)
}

/// The first `max_chars` of a transcript, cut on a character boundary.
fn transcript_preview(segments: &[TranscriptSegment], max_chars: usize) -> String {
    let mut preview = String::new();
    for segment in segments {
        if !preview.is_empty() {
            preview.push(' ');
        }
        preview.push_str(segment.text.trim());
        if preview.chars().count() >= max_chars {
            break;
        }
    }
    let truncated: String = preview.chars().take(max_chars).collect();
    if truncated.chars().count() < preview.chars().count() {
        format!("{truncated}…")
    } else {
        truncated
    }
}

/// `context` characters of text either side of the first match of `needle`.
fn excerpt_around(text: &str, needle_lower: &str, context: usize) -> String {
    let lowered = text.to_lowercase();
    let Some(byte_index) = lowered.find(needle_lower) else {
        return text.chars().take(context * 2).collect();
    };
    // Byte offsets from the lowercased copy can land mid-character in the
    // original for any non-ASCII text, so convert to a character index before
    // slicing anything.
    let char_index = lowered[..byte_index].chars().count();
    let start = char_index.saturating_sub(context);
    let chars: Vec<char> = text.chars().collect();
    let end = (char_index + needle_lower.chars().count() + context).min(chars.len());
    let mut excerpt = String::new();
    if start > 0 {
        excerpt.push('…');
    }
    excerpt.extend(&chars[start..end]);
    if end < chars.len() {
        excerpt.push('…');
    }
    excerpt
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meetings::model::{MeetingSource, SegmentChannel};

    fn temp_vault(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "vox-meetings-{}-{}-{:?}",
            name,
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp vault");
        dir
    }

    fn segment(sequence: u64, text: &str) -> TranscriptSegment {
        TranscriptSegment {
            sequence,
            text: text.to_string(),
            start_seconds: sequence as f64,
            end_seconds: sequence as f64 + 1.0,
            channel: SegmentChannel::Mixed,
            no_speech_prob: 0.01,
            recorded_at: "2026-01-01T00:00:00Z".into(),
            cut_at_ceiling: false,
            original_text: None,
            romanized_text: None,
            translated_text: None,
            corrections: Vec::new(),
        }
    }

    #[test]
    fn a_created_meeting_round_trips_through_disk() {
        let vault = temp_vault("create");
        let store = MeetingStore::new(&vault);
        let meeting = Meeting::new("meeting-a".into(), "Weekly".into(), MeetingSource::Recorded);
        store.create(&meeting).expect("create");

        let loaded = store.load_meeting("meeting-a").expect("load");
        assert_eq!(loaded.title, "Weekly");
        assert_eq!(loaded.state, MeetingState::Recording);
        assert!(store.audio_dir("meeting-a").unwrap().exists());
    }

    #[test]
    fn the_transcript_cache_does_not_grow_with_the_vault() {
        // The cache is for the meeting being recorded, which re-reads its own
        // file on every append. Browsing a vault must not turn it into a
        // resident copy of every transcript opened since launch.
        let vault = temp_vault("cache-bound");
        let store = MeetingStore::new(&vault);
        for index in 0..(MAX_CACHED_TRANSCRIPTS * 3) {
            let id = format!("meeting-{index}");
            let meeting = Meeting::new(id.clone(), "Sync".into(), MeetingSource::Recorded);
            store.create(&meeting).expect("create");
            store
                .append_segments(&id, &[segment(1, "something was said")])
                .expect("append");
            store.load_transcript(&id).expect("load");
        }

        assert!(
            store.transcripts.lock_or_recover().len() <= MAX_CACHED_TRANSCRIPTS,
            "the cache held {} transcripts",
            store.transcripts.lock_or_recover().len()
        );

        // Still a cache, not a disabled one: the meeting just read is served
        // from memory rather than parsed again.
        let last = format!("meeting-{}", MAX_CACHED_TRANSCRIPTS * 3 - 1);
        assert!(store.transcripts.lock_or_recover().contains_key(&last));
    }

    #[test]
    fn appended_segments_come_back_ordered_and_deduplicated() {
        let vault = temp_vault("append");
        let store = MeetingStore::new(&vault);
        let meeting = Meeting::new("meeting-b".into(), "Sync".into(), MeetingSource::Recorded);
        store.create(&meeting).expect("create");

        store.append_segments("meeting-b", &[segment(3, "third")]).unwrap();
        store.append_segments("meeting-b", &[segment(1, "first")]).unwrap();
        // The same sequence arriving twice is the pipeline retrying, not a
        // second thing that was said.
        let total = store
            .append_segments("meeting-b", &[segment(3, "third again"), segment(2, "second")])
            .unwrap();

        assert_eq!(total, 3);
        let transcript = store.load_transcript("meeting-b").unwrap();
        let sequences: Vec<u64> = transcript.iter().map(|s| s.sequence).collect();
        assert_eq!(sequences, vec![1, 2, 3]);
        assert_eq!(transcript[2].text, "third");
    }

    #[test]
    fn an_id_that_climbs_out_of_the_vault_is_refused() {
        let vault = temp_vault("traversal");
        let store = MeetingStore::new(&vault);
        for bad in ["../escape", "..", "a/b", "meeting id", ""] {
            assert!(
                matches!(store.meeting_dir(bad), Err(MeetingStoreError::InvalidId(_))),
                "{bad:?} should be refused"
            );
        }
        assert!(store.meeting_dir("meeting-abc_123").is_ok());
    }

    #[test]
    fn deleting_a_meeting_takes_its_audio_with_it() {
        let vault = temp_vault("delete");
        let store = MeetingStore::new(&vault);
        let meeting = Meeting::new("meeting-c".into(), "Gone".into(), MeetingSource::Recorded);
        store.create(&meeting).expect("create");
        let audio = store.audio_dir("meeting-c").unwrap().join("audio.wav");
        fs::write(&audio, b"not really a wav").unwrap();

        store.delete_meeting("meeting-c").expect("delete");

        assert!(!audio.exists());
        assert!(!store.meeting_dir("meeting-c").unwrap().exists());
        assert!(matches!(
            store.delete_meeting("meeting-c"),
            Err(MeetingStoreError::NotFound(_))
        ));
    }

    #[test]
    fn recovery_completes_an_interrupted_meeting_and_leaves_finished_ones_alone() {
        let vault = temp_vault("recover");
        let store = MeetingStore::new(&vault);

        let crashed = Meeting::new("meeting-d".into(), "Crashed".into(), MeetingSource::Recorded);
        store.create(&crashed).expect("create");
        store.append_segments("meeting-d", &[segment(1, "we were saying")]).unwrap();

        let mut finished = Meeting::new("meeting-e".into(), "Fine".into(), MeetingSource::Recorded);
        finished.state = MeetingState::Completed;
        finished.duration_seconds = 42.0;
        store.create(&finished).expect("create");
        store.save_meeting(&finished).expect("save completed");

        let recovered = store.recover_interrupted().expect("recover");

        assert_eq!(recovered, vec!["meeting-d".to_string()]);
        let d = store.load_meeting("meeting-d").unwrap();
        assert_eq!(d.state, MeetingState::Completed);
        assert_eq!(d.segment_count, 1);
        assert!(d.error.is_some(), "recovery should say the meeting was interrupted");
        let e = store.load_meeting("meeting-e").unwrap();
        assert_eq!(e.duration_seconds, 42.0);
        assert!(e.error.is_none());
    }

    #[test]
    fn recovery_recomputes_duration_from_the_audio_that_reached_disk() {
        let vault = temp_vault("recover-duration");
        let store = MeetingStore::new(&vault);
        let meeting = Meeting::new("meeting-f".into(), "Crashed".into(), MeetingSource::Recorded);
        store.create(&meeting).expect("create");

        // Two seconds of 16 kHz mono, written as one checkpoint.
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let chunk = store.audio_dir("meeting-f").unwrap().join("chunk_000000.wav");
        let mut writer = hound::WavWriter::create(&chunk, spec).unwrap();
        for _ in 0..32_000 {
            writer.write_sample(0i16).unwrap();
        }
        writer.finalize().unwrap();

        store.recover_interrupted().expect("recover");

        let recovered = store.load_meeting("meeting-f").unwrap();
        assert!((recovered.duration_seconds - 2.0).abs() < 0.01);
    }

    #[test]
    fn search_finds_a_phrase_and_returns_it_in_context() {
        let vault = temp_vault("search");
        let store = MeetingStore::new(&vault);
        let meeting = Meeting::new("meeting-g".into(), "Planning".into(), MeetingSource::Recorded);
        store.create(&meeting).expect("create");
        store
            .append_segments(
                "meeting-g",
                &[segment(1, "we should ship the migration before the holiday freeze")],
            )
            .unwrap();

        let hits = store.search("MIGRATION", 10).expect("search");

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].meeting_id, "meeting-g");
        assert!(hits[0].excerpt.contains("migration"));
        assert!(store.search("   ", 10).unwrap().is_empty());
    }

    #[test]
    fn search_context_does_not_split_a_multibyte_character() {
        // The needle's byte offset comes from a lowercased copy; slicing the
        // original on it would panic the moment anything non-ASCII precedes
        // the match.
        let excerpt = excerpt_around("मुझे migration चाहिए", "migration", 5);
        assert!(excerpt.contains("migration"));
    }

    #[test]
    fn a_listing_skips_an_unreadable_meeting_instead_of_failing() {
        let vault = temp_vault("corrupt");
        let store = MeetingStore::new(&vault);
        let good = Meeting::new("meeting-h".into(), "Good".into(), MeetingSource::Recorded);
        store.create(&good).expect("create");

        let bad_dir = store.meetings_dir().join("meeting-i");
        fs::create_dir_all(&bad_dir).unwrap();
        fs::write(bad_dir.join(MEETING_FILE), b"{ not json").unwrap();

        let meetings = store.list_meetings().expect("list");
        assert_eq!(meetings.len(), 1);
        assert_eq!(meetings[0].id, "meeting-h");
    }

    #[test]
    fn a_display_listing_reports_summary_state_and_a_preview() {
        let vault = temp_vault("display");
        let store = MeetingStore::new(&vault);
        let meeting = Meeting::new("meeting-j".into(), "Retro".into(), MeetingSource::Recorded);
        store.create(&meeting).expect("create");
        store.append_segments("meeting-j", &[segment(1, "what went well")]).unwrap();
        let mut summary = MeetingSummary::pending("meeting-j", "default");
        summary.status = SummaryStatus::Completed;
        summary.markdown = Some("# Retro".into());
        store.save_summary(&summary).unwrap();

        let items = store.list_for_display().expect("list");

        assert_eq!(items.len(), 1);
        assert!(items[0].has_summary);
        assert_eq!(items[0].summary_status, Some(SummaryStatus::Completed));
        assert_eq!(items[0].preview, "what went well");
    }

    #[test]
    fn a_preview_is_cut_on_a_character_boundary() {
        let segments = vec![segment(1, &"नमस्ते ".repeat(60))];
        let preview = transcript_preview(&segments, 20);
        assert!(preview.chars().count() <= 21, "20 chars plus the ellipsis");
        assert!(preview.ends_with('…'));
    }

    #[test]
    fn an_atomic_write_leaves_no_temporary_file_behind() {
        let vault = temp_vault("atomic");
        let target = vault.join("thing.json");
        write_atomic(&target, b"{}").expect("write");
        assert_eq!(fs::read_to_string(&target).unwrap(), "{}");
        let leftovers: Vec<_> = fs::read_dir(&vault)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().contains("tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temporary file was not renamed away");
    }

    #[test]
    fn notes_default_to_empty_rather_than_missing() {
        let vault = temp_vault("notes");
        let store = MeetingStore::new(&vault);
        let meeting = Meeting::new("meeting-k".into(), "Notes".into(), MeetingSource::Recorded);
        store.create(&meeting).expect("create");
        assert_eq!(store.load_notes("meeting-k").unwrap(), "");
        store.save_notes("meeting-k", "- follow up with ops").unwrap();
        assert_eq!(store.load_notes("meeting-k").unwrap(), "- follow up with ops");
    }

    fn diagnostic(sequence: u64) -> SegmentDiagnostics {
        use crate::meetings::telemetry::{SegmentStatus, DIAGNOSTICS_VERSION};
        SegmentDiagnostics {
            version: DIAGNOSTICS_VERSION,
            sequence,
            start_seconds: sequence as f64,
            end_seconds: sequence as f64 + 1.0,
            channel: SegmentChannel::Microphone,
            forced_split: false,
            end_reason: crate::meetings::speech_state::TurnEnd::Silence,
            hangover_ms: 400,
            voiced_seconds: 0.9,
            total_seconds: 1.0,
            no_speech_prob: Some(0.05),
            queue_wait_ms: 1,
            lock_wait_ms: 0,
            model_load_ms: 0,
            model_reloaded: false,
            decode_ms: 100,
            post_ms: 1,
            persist_ms: 1,
            model: "ggml-small.bin".into(),
            language: Some("en".into()),
            expensive_script_profile: false,
            audio_ctx: None,
            status: SegmentStatus::Kept,
            rejection: None,
            error: None,
            text_chars: 12,
            queue_depth_after: 0,
        }
    }

    #[test]
    fn the_live_transcript_is_kept_when_a_final_pass_replaces_it() {
        let store = MeetingStore::new(temp_vault("live-archive"));
        store
            .create(&Meeting::new("meeting-p".into(), "Passes".into(), MeetingSource::Recorded))
            .unwrap();
        store
            .save_transcript("meeting-p", &[segment(0, "live text")])
            .unwrap();

        assert!(store.archive_live_transcript("meeting-p").unwrap());
        store
            .save_transcript("meeting-p", &[segment(0, "final text")])
            .unwrap();

        assert_eq!(store.load_transcript("meeting-p").unwrap()[0].text, "final text");
        let live = store
            .load_live_transcript("meeting-p")
            .unwrap()
            .expect("the live transcript must survive the pass that replaced it");
        assert_eq!(live[0].text, "live text");
    }

    #[test]
    fn a_second_final_pass_does_not_archive_over_the_live_transcript() {
        // Archiving again would replace the live transcript with the first
        // final one, which is the only thing worth comparing against.
        let store = MeetingStore::new(temp_vault("live-archive-twice"));
        store
            .create(&Meeting::new("meeting-p".into(), "Twice".into(), MeetingSource::Recorded))
            .unwrap();
        store
            .save_transcript("meeting-p", &[segment(0, "live text")])
            .unwrap();

        assert!(store.archive_live_transcript("meeting-p").unwrap());
        store
            .save_transcript("meeting-p", &[segment(0, "first final")])
            .unwrap();
        assert!(
            !store.archive_live_transcript("meeting-p").unwrap(),
            "a second pass must not archive again"
        );

        let live = store.load_live_transcript("meeting-p").unwrap().unwrap();
        assert_eq!(live[0].text, "live text");
    }

    #[test]
    fn a_meeting_with_no_final_pass_has_no_archive_and_that_is_not_a_loss() {
        // `None` means transcript.json *is* the live one, not that it is gone.
        let store = MeetingStore::new(temp_vault("live-none"));
        store
            .create(&Meeting::new("meeting-p".into(), "Live".into(), MeetingSource::Recorded))
            .unwrap();
        store.save_transcript("meeting-p", &[segment(0, "only")]).unwrap();
        assert!(store.load_live_transcript("meeting-p").unwrap().is_none());
    }

    #[test]
    fn an_empty_transcript_is_not_archived_over_the_real_one() {
        // `create` writes an empty transcript.json, so archiving
        // unconditionally would fill the once-only slot with nothing and hide
        // the live transcript from every later pass.
        let store = MeetingStore::new(temp_vault("live-empty"));
        store
            .create(&Meeting::new("meeting-p".into(), "Empty".into(), MeetingSource::Recorded))
            .unwrap();
        assert!(!store.archive_live_transcript("meeting-p").unwrap());
        assert!(store.load_live_transcript("meeting-p").unwrap().is_none());

        // Once there is something to keep, the slot is still free.
        store.save_transcript("meeting-p", &[segment(0, "live")]).unwrap();
        assert!(store.archive_live_transcript("meeting-p").unwrap());
        assert_eq!(
            store.load_live_transcript("meeting-p").unwrap().unwrap()[0].text,
            "live"
        );
    }

    #[test]
    fn a_meeting_records_what_produced_its_transcript_without_naming_a_machine() {
        use crate::meetings::model::{TranscriptProvenance, TranscriptionPass};
        let store = MeetingStore::new(temp_vault("provenance"));
        let mut meeting =
            Meeting::new("meeting-p".into(), "Prov".into(), MeetingSource::Recorded);
        meeting.transcript = Some(TranscriptProvenance {
            pass: TranscriptionPass::Final,
            engine: "whisper".into(),
            model: "ggml-large-v3-turbo.bin".into(),
            language: Some("hi".into()),
            profile: "BeamSearch { beam_size: 5, patience: 1.0 }".into(),
            completed_at: "2026-09-19T10:00:00Z".into(),
        });
        store.create(&meeting).unwrap();

        let loaded = store.load_meeting("meeting-p").unwrap();
        let provenance = loaded.transcript.expect("provenance survives a round trip");
        assert_eq!(provenance.pass, TranscriptionPass::Final);
        assert_eq!(provenance.model, "ggml-large-v3-turbo.bin");
        assert!(!provenance.model.contains('/'), "a filename, never a path");
    }

    #[test]
    fn segment_diagnostics_append_rather_than_rewriting_the_file() {
        let store = MeetingStore::new(temp_vault("diag-append"));
        store
            .create(&Meeting::new("meeting-d".into(), "Diag".into(), MeetingSource::Recorded))
            .unwrap();

        for sequence in 0..5 {
            store
                .append_segment_diagnostics("meeting-d", &diagnostic(sequence))
                .unwrap();
        }
        let loaded = store.load_segment_diagnostics("meeting-d").unwrap();
        assert_eq!(loaded.len(), 5);
        assert_eq!(loaded[4].sequence, 4);

        // One line per record, which is what makes the thousandth append cost
        // what the first did.
        let path = store
            .meeting_dir("meeting-d")
            .unwrap()
            .join(SEGMENT_DIAGNOSTICS_FILE);
        let raw = fs::read_to_string(path).unwrap();
        assert_eq!(raw.lines().filter(|l| !l.trim().is_empty()).count(), 5);
    }

    #[test]
    fn a_torn_final_line_costs_that_record_and_not_the_log() {
        // The usual reason a diagnostics file is truncated is the crash whose
        // diagnostics somebody is trying to read.
        use std::io::Write;
        let store = MeetingStore::new(temp_vault("diag-torn"));
        store
            .create(&Meeting::new("meeting-t".into(), "Torn".into(), MeetingSource::Recorded))
            .unwrap();
        store
            .append_segment_diagnostics("meeting-t", &diagnostic(0))
            .unwrap();

        let path = store
            .meeting_dir("meeting-t")
            .unwrap()
            .join(SEGMENT_DIAGNOSTICS_FILE);
        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(b"{\"version\":1,\"sequence\":1,\"sta").unwrap();
        drop(file);

        let loaded = store.load_segment_diagnostics("meeting-t").unwrap();
        assert_eq!(loaded.len(), 1, "the complete record must survive");
        assert_eq!(loaded[0].sequence, 0);
    }

    #[test]
    fn diagnostics_for_a_meeting_that_has_none_read_as_absent_not_as_an_error() {
        let store = MeetingStore::new(temp_vault("diag-missing"));
        store
            .create(&Meeting::new("meeting-n".into(), "None".into(), MeetingSource::Recorded))
            .unwrap();
        assert!(store.load_diagnostics("meeting-n").unwrap().is_none());
        assert!(store.load_segment_diagnostics("meeting-n").unwrap().is_empty());
    }

    #[test]
    fn the_rollup_round_trips_through_the_store() {
        use crate::meetings::telemetry::{CaptureHealth, MeetingDiagnostics, StopFacts};
        let store = MeetingStore::new(temp_vault("diag-rollup"));
        store
            .create(&Meeting::new("meeting-r".into(), "Roll".into(), MeetingSource::Recorded))
            .unwrap();

        let rollup = MeetingDiagnostics::summarize(
            "meeting-r",
            &[diagnostic(0), diagnostic(1)],
            CaptureHealth {
                recording_seconds: 120.0,
                microphone_opened: true,
                microphone_heard: true,
                ..CaptureHealth::default()
            },
            StopFacts {
                model: "ggml-small.bin".into(),
                drain_seconds: 3.0,
                drain_completed: true,
                ..StopFacts::default()
            },
        );
        store.save_diagnostics(&rollup).unwrap();
        assert_eq!(store.load_diagnostics("meeting-r").unwrap(), Some(rollup));
    }

    #[test]
    fn deleting_a_meeting_takes_its_diagnostics_with_it() {
        // A diagnostics file names models, timings and segment boundaries of a
        // conversation. Deleting the meeting has to delete it too.
        let store = MeetingStore::new(temp_vault("diag-delete"));
        store
            .create(&Meeting::new("meeting-x".into(), "Gone".into(), MeetingSource::Recorded))
            .unwrap();
        store
            .append_segment_diagnostics("meeting-x", &diagnostic(0))
            .unwrap();
        let dir = store.meeting_dir("meeting-x").unwrap();
        assert!(dir.join(SEGMENT_DIAGNOSTICS_FILE).exists());

        store.delete_meeting("meeting-x").unwrap();
        assert!(!dir.exists());
    }
}
