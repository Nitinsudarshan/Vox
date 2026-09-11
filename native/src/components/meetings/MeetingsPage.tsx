import React from 'react';
import { listen } from '@tauri-apps/api/event';
import { Import, Search, Users } from 'lucide-react';

import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { PageHeader } from '@/components/common/PageHeader';
import { MeetingRecorder } from './MeetingRecorder';
import { MeetingModelGate } from './MeetingModelGate';
import { MeetingList } from './MeetingList';
import { MeetingDetail } from './MeetingDetail';
import * as meetings from '@/lib/meetings';
import { meetingErrorMessage } from '@/lib/meetings';
import {
  MEETING_EVENTS,
  type MeetingDetail as MeetingDetailData,
  type MeetingListItem,
  type MeetingDevices,
  type MeetingRecordingStatus,
  type MeetingTemplate,
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
 */
interface MeetingsPageProps {
  /** Opens Settings › Speech, so a missing model can be installed from here. */
  onOpenSpeechSettings?: () => void;
}

export const MeetingsPage: React.FC<MeetingsPageProps> = ({ onOpenSpeechSettings }) => {
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
  /** Bumped to force the model gate to re-read what is installed. */
  const [modelGateNonce, setModelGateNonce] = React.useState(0);
  /** The saved microphone/output choice. Empty means "let Vox decide". */
  const [devices, setDevices] = React.useState<MeetingDevices>({});
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
  }, [notify]);

  const refreshDetail = React.useCallback(async (meetingId: string) => {
    try {
      const next = await meetings.getMeeting(meetingId);
      // Guard against a slow fetch landing after the user moved on.
      if (selectedIdRef.current === meetingId) setDetail(next);
    } catch (error) {
      if (selectedIdRef.current === meetingId) setDetail(null);
    }
  }, []);

  React.useEffect(() => {
    void refreshList();
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
    // Templates and the initial list are read once; everything after is driven
    // by events and by explicit refreshes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  React.useEffect(() => {
    if (selectedId) void refreshDetail(selectedId);
    else setDetail(null);
  }, [selectedId, refreshDetail]);

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

  const handleRetranscribe = () =>
    run(async () => {
      if (!selectedId) return;
      await meetings.retranscribeMeeting(selectedId);
      await Promise.all([refreshList(), refreshDetail(selectedId)]);
      notify('info', 'Transcribed again with the current speech model.');
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

  return (
    <div className="flex flex-col h-full min-h-0">
      <PageHeader
        variant="minimal"
        kicker="On this machine"
        title="Meetings"
        description="Record both sides of a call, transcribe it here, and turn it into a report."
        badge={{ label: 'Local', icon: Users, variant: 'emerald' }}
      />

      <div className="shrink-0 space-y-3">
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
        <MeetingModelGate
          key={modelGateNonce}
          onOpenSpeechSettings={onOpenSpeechSettings}
          onModelInstalled={() => notify('info', 'Speech model installed. Recording is ready.')}
        />
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

      <div className="flex-1 min-h-0 grid grid-cols-1 lg:grid-cols-[320px_1fr] gap-4 mt-4">
        <aside className="flex flex-col min-h-0 border border-border rounded-xl p-3">
          <div className="flex items-center gap-2 mb-3 shrink-0">
            <div className="relative flex-1">
              <Search className="absolute left-2 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-muted-foreground" />
              <Input
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                placeholder="Search meetings…"
                aria-label="Search meetings"
                className="h-8 text-xs pl-7"
              />
            </div>
            <Button
              size="icon"
              variant="outline"
              onClick={handleImport}
              disabled={busy}
              aria-label="Import a recording"
              title="Import an existing recording"
              className="h-8 w-8 shrink-0"
            >
              <Import className="w-4 h-4" />
            </Button>
          </div>
          <div className="flex-1 min-h-0 overflow-y-auto pr-1">
            <MeetingList
              meetings={visible}
              selectedId={selectedId}
              onSelect={setSelectedId}
              loading={listLoading}
            />
          </div>
        </aside>

        <section className="min-h-0 border border-border rounded-xl p-4">
          {detail ? (
            <MeetingDetail
              detail={detail}
              templates={templates}
              templateId={templateId}
              onTemplateChange={setTemplateId}
              summaryProgress={summaryProgress}
              live={live}
              busy={busy}
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
              onRetranscribe={handleRetranscribe}
              onGenerateSummary={handleGenerate}
              onCancelSummary={handleCancelSummary}
              onSaveSummary={handleSaveSummary}
              onPromote={handlePromote}
            />
          ) : (
            <div className="h-full flex items-center justify-center">
              <p className="text-xs text-muted-foreground">
                {list.length === 0
                  ? 'Record a meeting to get started.'
                  : 'Select a meeting to read it.'}
              </p>
            </div>
          )}
        </section>
      </div>
    </div>
  );
};
