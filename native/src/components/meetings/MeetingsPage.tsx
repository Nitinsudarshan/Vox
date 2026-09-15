import React from 'react';
import { listen } from '@tauri-apps/api/event';
import { CalendarDays, Radio } from 'lucide-react';

import { Button } from '@/components/ui/button';
import { PageHeader } from '@/components/common/PageHeader';
import { MeetingRecorder } from './MeetingRecorder';
import { MeetingModelGate } from './MeetingModelGate';
import { MeetingIndex } from './MeetingIndex';
import { MeetingView } from './MeetingView';
import { RetranscribeDialog } from './RetranscribeDialog';
import { CalendarDialog } from './calendar/CalendarDialog';
import { SeriesDialog } from './series/SeriesDialog';
import { EventDetailsDialog } from './calendar/EventDetailsDialog';
import * as meetings from '@/lib/meetings';
import { meetingErrorMessage, type RetranscribeOverrides } from '@/lib/meetings';
import * as calendar from '@/lib/calendar';
import type { CalendarAccount, CalendarEvent, DayAgenda } from '@/types/calendar';
import {
  MEETING_EVENTS,
  type MeetingDetail as MeetingDetailData,
  type MeetingListItem,
  type MeetingDevices,
  type MeetingRecordingStatus,
  type MeetingSeriesSummary,
  type MeetingTemplate,
  type SeriesOccurrence,
  type SummaryProgress,
  type TranscriptSegment,
  type TranscriptionWarning,
} from '@/types/meetings';

/** Polls the recording clock while a meeting is in flight. */
const STATUS_POLL_MS = 1000;

/**
 * The Meetings surface: record, browse, read, summarise.
 *
 * State lives here and flows down. The backend is the source of truth for
 * everything persistent — this component holds only what it is currently
 * displaying, and re-reads after every mutation rather than patching its own
 * copy.
 *
 * It renders one of two things, never both: the index, or one meeting. The
 * version this replaces rendered both at once in a two-column layout, which is
 * how a transcript came to be read in a half-width pane underneath a recorder
 * card and a model-install card that were relevant to neither reading nor
 * choosing.
 */
interface MeetingsPageProps {
  /** Opens Settings › Speech, so a missing model can be installed from here. */
  onOpenSpeechSettings?: () => void;
  /** Opens Settings › AI Models & STT, for a report that has no provider. */
  onOpenProviderSettings?: () => void;
  /** Opens Settings › Calendar, for connecting a Google account. */
  onOpenCalendarSettings?: () => void;
}

export const MeetingsPage: React.FC<MeetingsPageProps> = ({
  onOpenSpeechSettings,
  onOpenProviderSettings,
  onOpenCalendarSettings,
}) => {
  const [status, setStatus] = React.useState<MeetingRecordingStatus>({
    active: false,
    elapsed_seconds: 0,
    microphone_active: false,
    system_audio_active: false,
    microphone_heard: false,
    system_audio_heard: false,
    segments_queued: 0,
    segments_completed: 0,
    segments_dropped: 0,
  });
  const [list, setList] = React.useState<MeetingListItem[]>([]);
  const [listLoading, setListLoading] = React.useState(true);
  const [selectedId, setSelectedId] = React.useState<string | null>(null);
  const [detail, setDetail] = React.useState<MeetingDetailData | null>(null);
  const [templates, setTemplates] = React.useState<MeetingTemplate[]>([]);
  const [templateId, setTemplateId] = React.useState('general');
  const [summaryProgress, setSummaryProgress] = React.useState<SummaryProgress | null>(null);
  const [query, setQuery] = React.useState('');
  const [busy, setBusy] = React.useState(false);
  const [translatingTranscript, setTranslatingTranscript] = React.useState(false);
  const [detectingSpeakers, setDetectingSpeakers] = React.useState(false);
  const [retranscribeOpen, setRetranscribeOpen] = React.useState(false);
  const [calendarOpen, setCalendarOpen] = React.useState(false);
  /** The meeting whose details are open. One dialog serves both surfaces. */
  const [openEvent, setOpenEvent] = React.useState<CalendarEvent | null>(null);
  const [seriesDialogOpen, setSeriesDialogOpen] = React.useState(false);
  const [allSeries, setAllSeries] = React.useState<MeetingSeriesSummary[]>([]);
  const [seriesOccurrences, setSeriesOccurrences] = React.useState<SeriesOccurrence[]>([]);
  const [seriesLoading, setSeriesLoading] = React.useState(false);
  const [retranscribing, setRetranscribing] = React.useState(false);
  /** Bumped to force the model gate to re-read what is installed. */
  const [modelGateNonce, setModelGateNonce] = React.useState(0);
  /** The saved microphone/output choice. Empty means "let Vox decide". */
  const [devices, setDevices] = React.useState<MeetingDevices>({});
  const [agenda, setAgenda] = React.useState<DayAgenda[]>([]);
  const [accounts, setAccounts] = React.useState<CalendarAccount[]>([]);
  const [agendaSyncing, setAgendaSyncing] = React.useState(false);
  const [message, setMessage] = React.useState<{ kind: 'info' | 'error'; text: string } | null>(
    null,
  );

  const selectedIdRef = React.useRef<string | null>(null);
  selectedIdRef.current = selectedId;

  const notify = React.useCallback((kind: 'info' | 'error', text: string) => {
    setMessage({ kind, text });
    window.setTimeout(() => setMessage(null), 6000);
  }, []);

  const refreshList = React.useCallback(async () => {
    try {
      setList(await meetings.listMeetings());
    } catch (error) {
      notify('error', meetingErrorMessage(error));
    } finally {
      setListLoading(false);
    }
    try {
      // The catalogue, not the membership: a meeting carries its series id, and
      // this is what turns that id into a name.
      setAllSeries((await meetings.listMeetingSeries()) ?? []);
    } catch {
      setAllSeries([]);
    }
  }, [notify]);

  const refreshDetail = React.useCallback(async (meetingId: string) => {
    try {
      const next = await meetings.getMeeting(meetingId);
      // Guard against a slow fetch landing after the user moved on.
      if (selectedIdRef.current === meetingId) setDetail(next);
    } catch {
      if (selectedIdRef.current === meetingId) setDetail(null);
    }
  }, []);

  const refreshAgenda = React.useCallback(async () => {
    try {
      // `?? []` for the same reason the device picker does it: a command that
      // resolves to nothing must leave the agenda empty rather than hand the
      // index an undefined to read through.
      setAgenda((await calendar.getCalendarAgenda()) ?? []);
    } catch {
      // A calendar that cannot be read is not worth a banner on the meetings
      // page. Settings › Calendar is where the failure belongs.
      setAgenda([]);
    }
    try {
      // The accounts are what colour the agenda's rows, so they are read
      // alongside it rather than once at mount: connecting a second account in
      // Settings and coming back must not leave both calendars the same shade.
      setAccounts((await calendar.listCalendarAccounts()) ?? []);
    } catch {
      setAccounts([]);
    }
  }, []);

  /** Re-reads every connected calendar, from the meetings page itself. */
  const syncCalendars = React.useCallback(async () => {
    setAgendaSyncing(true);
    try {
      await calendar.syncCalendars();
      await refreshAgenda();
    } catch {
      // Per-account failures are recorded against the account and shown in
      // Settings › Calendar; a sync that fails entirely leaves the cached
      // agenda on screen, which is the useful thing to do with it.
    } finally {
      setAgendaSyncing(false);
    }
  }, [refreshAgenda]);

  React.useEffect(() => {
    void refreshList();
    void refreshAgenda();
    void meetings
      .listTemplates()
      .then((available) => {
        setTemplates(available);
        if (available.length > 0 && !available.some((t) => t.id === templateId)) {
          setTemplateId(available[0].id);
        }
      })
      .catch(() => setTemplates([]));
    void meetings.getRecordingStatus().then(setStatus).catch(() => undefined);
    // `?? {}` rather than the raw answer: a command that fails, or a settings
    // file written before this existed, must leave the picker on its defaults
    // and not hand the recorder an undefined to read through.
    void meetings
      .getMeetingDevices()
      .then((saved) => setDevices(saved ?? {}))
      .catch(() => undefined);

    // Sync in the background. The agenda above has already rendered from the
    // cache, so a slow or failing sync costs nothing the user is waiting on.
    void syncCalendars();
    // Templates and the initial list are read once; everything after is driven
    // by events and by explicit refreshes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  React.useEffect(() => {
    if (selectedId) void refreshDetail(selectedId);
    else setDetail(null);
  }, [selectedId, refreshDetail]);

  /** The series the open meeting belongs to, as a record rather than an id. */
  const currentSeries = React.useMemo(
    () => allSeries.find((entry) => entry.id === detail?.meeting.series_id) ?? null,
    [allSeries, detail?.meeting.series_id],
  );

  const refreshSeriesOccurrences = React.useCallback(async (seriesId: string) => {
    setSeriesLoading(true);
    try {
      setSeriesOccurrences((await meetings.getMeetingSeries(seriesId)) ?? []);
    } catch {
      setSeriesOccurrences([]);
    } finally {
      setSeriesLoading(false);
    }
  }, []);

  React.useEffect(() => {
    const seriesId = detail?.meeting.series_id;
    if (!seriesId) {
      setSeriesOccurrences([]);
      return;
    }
    void refreshSeriesOccurrences(seriesId);
  }, [detail?.meeting.series_id, refreshSeriesOccurrences]);

  // A recording's clock is not event-driven, so the status is polled while one
  // is running and left alone when none is.
  React.useEffect(() => {
    if (!status.active) return undefined;
    const timer = window.setInterval(() => {
      void meetings.getRecordingStatus().then(setStatus).catch(() => undefined);
    }, STATUS_POLL_MS);
    return () => window.clearInterval(timer);
  }, [status.active]);

  React.useEffect(() => {
    const subscriptions = [
      listen<MeetingRecordingStatus>(MEETING_EVENTS.state, (event) => {
        setStatus(event.payload);
        void refreshList();
      }),
      listen<TranscriptSegment>(MEETING_EVENTS.segment, (event) => {
        const segment = event.payload;
        setDetail((current) => {
          if (!current || current.meeting.id !== selectedIdRef.current) return current;
          if (current.segments.some((existing) => existing.sequence === segment.sequence)) {
            return current;
          }
          const segments = [...current.segments, segment].sort(
            (a, b) => a.sequence - b.sequence,
          );
          return { ...current, segments };
        });
      }),
      listen<SummaryProgress>(MEETING_EVENTS.summaryProgress, (event) => {
        if (event.payload.meeting_id !== selectedIdRef.current) return;
        setSummaryProgress(event.payload);
        if (
          event.payload.status === 'completed' ||
          event.payload.status === 'failed' ||
          event.payload.status === 'cancelled'
        ) {
          setSummaryProgress(null);
          void refreshDetail(event.payload.meeting_id);
          void refreshList();
        }
      }),
      listen<TranscriptionWarning>(MEETING_EVENTS.transcriptionWarning, (event) => {
        notify('error', event.payload.message);
      }),
    ];
    return () => {
      subscriptions.forEach((subscription) => {
        void subscription.then((off) => off());
      });
    };
  }, [notify, refreshDetail, refreshList]);

  const run = React.useCallback(
    async (action: () => Promise<void>) => {
      setBusy(true);
      try {
        await action();
      } catch (error) {
        notify('error', meetingErrorMessage(error));
      } finally {
        setBusy(false);
      }
    },
    [notify],
  );

  const handleStart = () =>
    run(async () => {
      try {
        const meeting = await meetings.startMeeting(undefined, undefined, devices);
        setSelectedId(meeting.id);
        await refreshList();
      } catch (error) {
        // Re-read the catalogue on the way past. The gate is rendered from a
        // snapshot taken when the page mounted, and the most likely reason a
        // start fails is that the snapshot is stale — a model was deleted, or
        // installed in Settings without coming back here.
        setModelGateNonce((n) => n + 1);
        throw error;
      }
    });

  const handleStop = () =>
    run(async () => {
      const meeting = await meetings.stopMeeting();
      setSelectedId(meeting.id);
      await Promise.all([refreshList(), refreshDetail(meeting.id)]);
    });

  const handleImport = () =>
    run(async () => {
      const path = await meetings.pickAudioFile();
      if (!path) return;
      notify('info', 'Transcribing the recording. This runs on this machine and takes a while.');
      const meeting = await meetings.importAudio(path);
      setSelectedId(meeting.id);
      await Promise.all([refreshList(), refreshDetail(meeting.id)]);
    });

  const handleRetranscribe = (overrides: RetranscribeOverrides) =>
    run(async () => {
      if (!selectedId) return;
      setRetranscribing(true);
      try {
        // The English pass is its own command so it can be run without
        // replacing a transcript. Here the two are one action, in order: the
        // transcript first, then English over whatever it produced.
        const { englishTrack, ...decode } = overrides;
        await meetings.retranscribeMeeting(selectedId, decode);
        await Promise.all([refreshList(), refreshDetail(selectedId)]);
        if (englishTrack) {
          notify('info', 'Transcribed. Now producing the English version…');
          const filled = await meetings.generateEnglishTrack(selectedId, decode);
          await refreshDetail(selectedId);
          notify('info', `Transcribed, and English added to ${filled} line(s).`);
        } else {
          notify('info', 'Transcribed again.');
        }
        setRetranscribeOpen(false);
      } finally {
        setRetranscribing(false);
      }
    });

  const handleTranslateTranscript = (targetLanguage?: string) =>
    run(async () => {
      if (!selectedId) return;
      setTranslatingTranscript(true);
      try {
        notify('info', 'Translating transcript…');
        await meetings.translateTranscript(selectedId, targetLanguage);
        await refreshDetail(selectedId);
        notify('info', 'Transcript translated.');
      } finally {
        setTranslatingTranscript(false);
      }
    });

  const handleDetectSpeakers = () =>
    run(async () => {
      if (!selectedId) return;
      setDetectingSpeakers(true);
      try {
        const report = await meetings.detectSpeakers(selectedId);
        await refreshDetail(selectedId);
        notify(
          'info',
          report.speakers.length === 0
            ? 'No voices were clear enough to group. Lines stay labelled by capture channel.'
            : `Found ${report.speakers.length} speaker(s) across ${report.attributed} line(s). Play each one to put a name to it.`,
        );
      } finally {
        setDetectingSpeakers(false);
      }
    });

  const handleRenameSpeaker = (speakerId: string, label: string) =>
    run(async () => {
      if (!selectedId) return;
      await meetings.renameSpeaker(selectedId, speakerId, label);
      await refreshDetail(selectedId);
    });

  const handleDelete = () =>
    run(async () => {
      if (!selectedId) return;
      await meetings.deleteMeeting(selectedId);
      setSelectedId(null);
      setDetail(null);
      await refreshList();
    });

  const handleGenerate = (force: boolean) =>
    run(async () => {
      if (!selectedId) return;
      await meetings.generateSummary({ meetingId: selectedId, templateId, force });
      setSummaryProgress({
        meeting_id: selectedId,
        status: 'processing',
        stage: 'Starting',
        fraction: 0,
      });
    });

  const handleCancelSummary = () =>
    run(async () => {
      if (!selectedId) return;
      await meetings.cancelSummary(selectedId);
      setSummaryProgress(null);
      await refreshDetail(selectedId);
    });

  const handleSaveSummary = (markdown: string) =>
    run(async () => {
      if (!selectedId) return;
      await meetings.saveSummary(selectedId, markdown);
      await Promise.all([refreshDetail(selectedId), refreshList()]);
    });

  /** Opens an invitation's video link in the browser. */
  const handleJoin = (event: CalendarEvent) =>
    run(async () => {
      setOpenEvent(null);
      if (!event.conference_url) return;
      await calendar.openCalendarLink(event.conference_url);
    });

  /** Opens the call and records it, in one press.
   *
   * The recording is named after the meeting rather than after the clock,
   * which is the whole reason the calendar is connected. */
  const handleJoinAndRecord = (event: CalendarEvent) =>
    run(async () => {
      setOpenEvent(null);
      if (event.conference_url) {
        await calendar.openCalendarLink(event.conference_url);
      }
      const meeting = await meetings.startMeeting(event.title, undefined, devices);
      setSelectedId(meeting.id);
      await refreshList();
    });

  const handlePromote = () =>
    run(async () => {
      if (!selectedId) return;
      await meetings.promoteToScribble(selectedId);
      notify('info', 'Added to your knowledge graph as a Scribble.');
    });

  const needle = query.trim().toLowerCase();
  const visible = needle
    ? list.filter(
        (meeting) =>
          meeting.title.toLowerCase().includes(needle) ||
          meeting.preview.toLowerCase().includes(needle),
      )
    : list;

  const live = status.active && status.meeting_id === selectedId;

  /** Whether this meeting holds a script the English view exists for. */
  const wantsEnglish = React.useMemo(
    () =>
      (detail?.segments ?? []).some(
        (segment) =>
          /[ऀ-ॿ]/.test(segment.text) || Boolean(segment.romanized_text?.trim()),
      ),
    [detail],
  );

  return (
    <div className="flex flex-col h-full min-h-0">
      {/* The banner belongs to whatever is on screen. On the index it is the
          meetings surface and the way into a new one; inside a meeting it is
          that meeting, because offering to start a second one from inside the
          first is an invitation to a mistake. */}
      {!detail && (
        <PageHeader
          title="Meetings &"
          highlightText="Calls"
          description="Join, record and transcribe your calls on this device, and turn them into reports."
          glowColor="emerald"
          compact
        >
          <div className="flex items-center gap-3 shrink-0 flex-wrap sm:flex-nowrap">
            <MeetingModelGate
              mode="status-only"
              key={`status-${modelGateNonce}`}
              onOpenSpeechSettings={onOpenSpeechSettings}
            />
            {accounts.length > 0 && (
              <Button
                variant="outline"
                size="sm"
                onClick={() => setCalendarOpen(true)}
                className="h-8 px-3 gap-2 shrink-0 text-xs font-medium"
              >
                <CalendarDays className="w-3.5 h-3.5" />
                <span>Calendar</span>
              </Button>
            )}
            {status.active ? (
              <div className="flex items-center gap-2 shrink-0 bg-red-500/10 border border-red-500/20 px-3 py-1.5 rounded-lg">
                <span className="flex h-2 w-2 relative">
                  <span className="animate-ping absolute inline-flex h-full w-full rounded-full bg-red-400 opacity-75" />
                  <span className="relative inline-flex rounded-full h-2 w-2 bg-red-500" />
                </span>
                <span className="text-xs font-mono font-medium text-red-500 dark:text-red-400">
                  Meeting in progress
                </span>
              </div>
            ) : (
              <Button
                onClick={handleStart}
                disabled={busy}
                className="h-8 px-3.5 gap-2 shrink-0 text-xs font-medium bg-emerald-600 hover:bg-emerald-500 text-white dark:bg-emerald-600 dark:hover:bg-emerald-500 shadow-xs transition-all active:scale-[0.98]"
              >
                <Radio className="w-3.5 h-3.5 text-white" />
                <span>Start meeting</span>
              </Button>
            )}
          </div>
        </PageHeader>
      )}

      <div className="shrink-0 space-y-3 empty:hidden">
        {/* Rendered whatever is on screen: it is the live meeting's own
            controls, and a recording you cannot stop from the meeting you are
            watching it produce is not a control surface. It draws nothing when
            no meeting is running. */}
        <MeetingRecorder
          status={status}
          busy={busy}
          onStart={handleStart}
          onPause={() => run(meetings.pauseMeeting)}
          onResume={() => run(meetings.resumeMeeting)}
          devices={devices}
          onDevicesChange={(next) => {
            // Optimistic: the picker must not lag the click, and a failed
            // write costs the choice sticking rather than this recording,
            // which passes the devices explicitly anyway.
            setDevices(next);
            void meetings.saveMeetingDevices(next).catch(() => undefined);
          }}
          onStop={handleStop}
        />
        {!detail && (
          <MeetingModelGate
            mode="card-only"
            key={`card-${modelGateNonce}`}
            onOpenSpeechSettings={onOpenSpeechSettings}
            onModelInstalled={() => notify('info', 'Speech model installed. Meetings are ready.')}
          />
        )}
        {message && (
          <p
            role="status"
            className={`text-xs ${
              message.kind === 'error' ? 'text-destructive' : 'text-muted-foreground'
            }`}
          >
            {message.text}
          </p>
        )}
      </div>

      <div className="flex-1 min-h-0 mt-3">
        {detail ? (
          <MeetingView
            detail={detail}
            templates={templates}
            templateId={templateId}
            onTemplateChange={setTemplateId}
            summaryProgress={summaryProgress}
            live={live}
            busy={busy}
            onBack={() => setSelectedId(null)}
            onRename={(title) =>
              run(async () => {
                if (!selectedId) return;
                await meetings.renameMeeting(selectedId, title);
                await Promise.all([refreshDetail(selectedId), refreshList()]);
              })
            }
            onDelete={handleDelete}
            onOpenFolder={() =>
              run(async () => {
                if (selectedId) await meetings.openMeetingFolder(selectedId);
              })
            }
            onRetranscribe={() => setRetranscribeOpen(true)}
            onGenerateSummary={handleGenerate}
            onCancelSummary={handleCancelSummary}
            onSaveSummary={handleSaveSummary}
            onPromote={handlePromote}
            onOpenProviderSettings={onOpenProviderSettings}
            onTranslateTranscript={handleTranslateTranscript}
            isTranslatingTranscript={translatingTranscript}
            onDetectSpeakers={handleDetectSpeakers}
            onRenameSpeaker={handleRenameSpeaker}
            detectingSpeakers={detectingSpeakers}
            seriesTitle={currentSeries?.title}
            seriesOccurrences={seriesOccurrences}
            seriesLoading={seriesLoading}
            onOpenMeeting={setSelectedId}
            onAddToSeries={() => setSeriesDialogOpen(true)}
          />
        ) : (
          <MeetingIndex
            meetings={visible}
            loading={listLoading}
            query={query}
            onQueryChange={setQuery}
            onSelect={setSelectedId}
            onImport={handleImport}
            busy={busy}
            agenda={agenda}
            accounts={accounts}
            agendaSyncing={agendaSyncing}
            onSyncCalendars={() => void syncCalendars()}
            onOpenEvent={setOpenEvent}
            onConnectCalendar={onOpenCalendarSettings}
          />
        )}
      </div>

      {detail && (
        <SeriesDialog
          open={seriesDialogOpen}
          onOpenChange={setSeriesDialogOpen}
          meetingId={detail.meeting.id}
          meetingTitle={detail.meeting.title}
          currentSeriesId={detail.meeting.series_id}
          onChanged={() => {
            void refreshList();
            if (selectedId) void refreshDetail(selectedId);
          }}
        />
      )}

      <CalendarDialog
        open={calendarOpen}
        onOpenChange={setCalendarOpen}
        agenda={agenda}
        accounts={accounts}
        syncing={agendaSyncing}
        onSync={() => void syncCalendars()}
        onOpenEvent={setOpenEvent}
      />

      <EventDetailsDialog
        event={openEvent}
        onOpenChange={(open) => {
          if (!open) setOpenEvent(null);
        }}
        connectedAccounts={accounts
          .filter((account) => account.enabled)
          .map((account) => account.email)}
        onJoin={handleJoin}
        onJoinAndRecord={handleJoinAndRecord}
        onOpenNotes={(meetingId) => {
          setOpenEvent(null);
          setCalendarOpen(false);
          setSelectedId(meetingId);
        }}
      />

      <RetranscribeDialog
        open={retranscribeOpen}
        onOpenChange={setRetranscribeOpen}
        onRun={handleRetranscribe}
        running={retranscribing}
        suggestEnglishTrack={wantsEnglish}
      />
    </div>
  );
};
