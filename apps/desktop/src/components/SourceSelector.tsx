import {
  Camera,
  Check,
  Gauge,
  HardDrive,
  Mic,
  Monitor,
  RectangleHorizontal,
  RefreshCw,
  Speaker,
  X,
} from 'lucide-react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { WebviewWindow } from '@tauri-apps/api/webviewWindow';
import { useEffect, useMemo, useRef, useState } from 'react';
import {
  getAudioMeter,
  getCaptureThumbnail,
  listAudioDevices,
  listCameraDevices,
  listCaptureSources,
  startRecording,
} from '@/ipc/client';
import type { AudioDevice, CameraDevice, CaptureSource } from '@/ipc/types';
import { ThemeSelector } from './ThemeSelector';

const browserSources: CaptureSource[] = [
  {
    id: 'display:mock',
    kind: 'display',
    title: 'Primary display',
    processName: null,
    bounds: { left: 0, top: 0, width: 1920, height: 1080 },
    dpi: 120,
    availability: 'available',
    thumbnailDataUrl: null,
  },
  {
    id: 'window:mock',
    kind: 'window',
    title: 'Product preview — Microsoft Edge',
    processName: 'msedge.exe',
    bounds: { left: 80, top: 60, width: 1440, height: 900 },
    dpi: 120,
    availability: 'available',
    thumbnailDataUrl: null,
  },
];

export function SourceSelector() {
  const [tab, setTab] = useState<'display' | 'window'>('display');
  const [sources, setSources] = useState<CaptureSource[]>([]);
  const [audio, setAudio] = useState<AudioDevice[]>([]);
  const [cameras, setCameras] = useState<CameraDevice[]>([]);
  const [sourceId, setSourceId] = useState('');
  const [microphoneId, setMicrophoneId] = useState('');
  const [cameraId, setCameraId] = useState('');
  const [systemAudio, setSystemAudio] = useState(true);
  const [fps, setFps] = useState<30 | 60>(60);
  const [countdown, setCountdown] = useState(3);
  const [shortcutProfile, setShortcutProfile] = useState(
    () => localStorage.getItem('kiri.shortcutProfile') ?? 'ctrl-shift',
  );
  const [counting, setCounting] = useState<number | null>(null);
  const [notice, setNotice] = useState('');
  const [microphonePeak, setMicrophonePeak] = useState(0);
  const [microphoneClipped, setMicrophoneClipped] = useState(false);
  const [loading, setLoading] = useState(true);
  const previewRef = useRef<HTMLVideoElement>(null);
  const projectPath = localStorage.getItem('kiri.captureProjectPath') ?? '';

  async function refresh() {
    setLoading(true);
    setNotice('');
    try {
      const [nextSources, nextAudio, nextCameras] = await Promise.all([
        listCaptureSources(),
        listAudioDevices(),
        listCameraDevices(),
      ]);
      const resolved = nextSources.length ? nextSources : browserSources;
      setSources(resolved);
      if ('__TAURI_INTERNALS__' in window) {
        void Promise.all(
          resolved
            .filter((source) => source.availability === 'available')
            .slice(0, 8)
            .map(async (source) => ({
              id: source.id,
              thumbnail: await getCaptureThumbnail(source.id).catch(() => null),
            })),
        ).then((thumbnails) =>
          setSources((current) =>
            current.map((source) => ({
              ...source,
              thumbnailDataUrl:
                thumbnails.find((item) => item.id === source.id)?.thumbnail ??
                source.thumbnailDataUrl,
            })),
          ),
        );
      }
      setAudio(nextAudio);
      setCameras(nextCameras);
      setSourceId((current) => current || resolved.find((value) => value.kind === tab)?.id || '');
      setMicrophoneId(
        (current) =>
          current ||
          nextAudio.find((device) => device.kind === 'microphone' && device.isDefault)?.id ||
          '',
      );
    } catch (error) {
      setNotice('Devices could not be refreshed. ' + String(error));
      setSources(browserSources);
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void refresh();
    // Native device discovery runs once when this window opens.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (!cameraId || !navigator.mediaDevices?.getUserMedia) {
      if (previewRef.current) previewRef.current.srcObject = null;
      return;
    }
    let stream: MediaStream | null = null;
    void navigator.mediaDevices
      .getUserMedia({ video: true, audio: false })
      .then((value) => {
        stream = value;
        if (previewRef.current) previewRef.current.srcObject = value;
      })
      .catch(() => setNotice('Camera preview is unavailable. Close other camera apps and retry.'));
    return () => stream?.getTracks().forEach((track) => track.stop());
  }, [cameraId]);

  useEffect(() => {
    if (!microphoneId) {
      setMicrophonePeak(0);
      setMicrophoneClipped(false);
      return;
    }
    const poll = async () => {
      try {
        const meter = await getAudioMeter(microphoneId);
        setMicrophonePeak(meter.peak);
        setMicrophoneClipped(meter.clipped);
      } catch {
        setMicrophonePeak(0);
      }
    };
    void poll();
    const timer = window.setInterval(() => void poll(), 150);
    return () => window.clearInterval(timer);
  }, [microphoneId]);
  const visibleSources = useMemo(
    () => sources.filter((source) => source.kind === tab),
    [sources, tab],
  );
  const selectedSource = sources.find((source) => source.id === sourceId);
  const estimatedGbHour = fps === 60 ? 7.3 : 4.6;
  const canStart =
    Boolean(projectPath) &&
    selectedSource?.availability === 'available' &&
    !loading &&
    counting === null;

  async function beginCountdown() {
    if (!canStart) return;
    if (countdown > 0) {
      for (let value = countdown; value > 0; value -= 1) {
        setCounting(value);
        await new Promise((resolve) => window.setTimeout(resolve, 1000));
      }
    }
    setCounting(0);
    try {
      await startRecording({
        projectPath,
        sourceId,
        microphoneId: microphoneId || null,
        systemAudio,
        cameraId: cameraId || null,
        fps,
      });
      const controller = await WebviewWindow.getByLabel('recording-controller');
      await controller?.show();
      await controller?.setFocus();
      await getCurrentWindow().hide();
    } catch (error) {
      setNotice('Recording did not start. ' + String(error));
      setCounting(null);
    }
  }

  function selectTab(next: 'display' | 'window') {
    setTab(next);
    setSourceId(
      sources.find((source) => source.kind === next && source.availability === 'available')?.id ??
        '',
    );
  }

  return (
    <main className="floating-shell capture-setup">
      {counting !== null && (
        <div className="countdown-overlay" role="status" aria-live="assertive">
          <span>{counting || 'REC'}</span>
          <small>{counting ? 'Recording starts shortly' : 'Starting capture'}</small>
        </div>
      )}
      <header className="floating-header" data-tauri-drag-region>
        <div>
          <strong>Capture setup</strong>
          <span>{projectPath || 'Create a recording project from Home first'}</span>
        </div>
        <ThemeSelector />
        <button className="icon-button" aria-label="Refresh devices" onClick={() => void refresh()}>
          <RefreshCw aria-hidden="true" />
        </button>
        <button
          className="icon-button"
          aria-label="Close"
          onClick={() => void getCurrentWindow().hide()}
        >
          <X aria-hidden="true" />
        </button>
      </header>
      <div className="source-tabs" role="tablist" aria-label="Capture source type">
        <button role="tab" aria-selected={tab === 'display'} onClick={() => selectTab('display')}>
          <Monitor aria-hidden="true" /> Displays
        </button>
        <button role="tab" aria-selected={tab === 'window'} onClick={() => selectTab('window')}>
          <RectangleHorizontal aria-hidden="true" /> Windows
        </button>
      </div>
      <section className="capture-body">
        <div className="source-list" role="tabpanel">
          {loading ? (
            <div className="source-loading">Discovering Windows capture sources…</div>
          ) : (
            visibleSources.map((source) => (
              <button
                key={source.id}
                className={source.id === sourceId ? 'source-item is-selected' : 'source-item'}
                disabled={source.availability !== 'available'}
                onClick={() => setSourceId(source.id)}
                title={
                  source.availability === 'available'
                    ? source.title
                    : source.title + ' is ' + source.availability
                }
              >
                <span className="source-thumbnail">
                  {source.kind === 'display' ? <Monitor /> : <RectangleHorizontal />}
                </span>
                <span>
                  <strong>{source.title || 'Untitled window'}</strong>
                  <small>
                    {source.processName ?? 'Windows display'} · {source.bounds.width}×
                    {source.bounds.height} · {Math.round((source.dpi / 96) * 100)}%
                  </small>
                </span>
                {source.id === sourceId && <Check aria-hidden="true" />}
                {source.availability !== 'available' && <em>{source.availability}</em>}
              </button>
            ))
          )}
        </div>
        <aside className="capture-options" aria-label="Recording configuration">
          <Setting icon={<Mic />} label="Microphone">
            <select value={microphoneId} onChange={(event) => setMicrophoneId(event.target.value)}>
              <option value="">Off</option>
              {audio
                .filter((device) => device.kind === 'microphone')
                .map((device) => (
                  <option key={device.id} value={device.id}>
                    {device.name}
                  </option>
                ))}
            </select>
          </Setting>
          <div
            className={microphoneClipped ? 'meter-row is-clipped' : 'meter-row'}
            aria-label={microphoneClipped ? 'Microphone clipping' : 'Microphone level'}
          >
            <span style={{ width: `${Math.round(microphonePeak * 100)}%` }} />
            <small>{microphoneId ? `${Math.round(microphonePeak * 100)}%` : 'Off'}</small>
          </div>
          <Setting icon={<Speaker />} label="System audio">
            <button
              className={systemAudio ? 'switch is-on' : 'switch'}
              role="switch"
              aria-checked={systemAudio}
              onClick={() => setSystemAudio((value) => !value)}
            >
              <span />
            </button>
          </Setting>
          <Setting icon={<Camera />} label="Camera">
            <select value={cameraId} onChange={(event) => setCameraId(event.target.value)}>
              <option value="">Off</option>
              {cameras.map((camera) => (
                <option key={camera.id} value={camera.id}>
                  {camera.name}
                </option>
              ))}
            </select>
          </Setting>
          {cameraId && (
            <div className="camera-preview">
              <video ref={previewRef} autoPlay muted playsInline />
              <span>Preview mirrored · source remains original</span>
            </div>
          )}
          <Setting icon={<Gauge />} label="Quality">
            <div className="segmented">
              <button className={fps === 30 ? 'is-active' : ''} onClick={() => setFps(30)}>
                30
              </button>
              <button className={fps === 60 ? 'is-active' : ''} onClick={() => setFps(60)}>
                60
              </button>
            </div>
          </Setting>
          <Setting icon={<Gauge />} label="Countdown">
            <select
              value={countdown}
              onChange={(event) => setCountdown(Number(event.target.value))}
            >
              <option value={0}>None</option>
              <option value={3}>3 seconds</option>
              <option value={5}>5 seconds</option>
            </select>
          </Setting>
          <Setting icon={<Gauge />} label="Shortcuts">
            <select
              value={shortcutProfile}
              onChange={(event) => {
                const value = event.target.value;
                setShortcutProfile(value);
                localStorage.setItem('kiri.shortcutProfile', value);
                window.dispatchEvent(new Event('kiri-shortcuts-changed'));
              }}
            >
              <option value="ctrl-shift">Ctrl + Shift</option>
              <option value="ctrl-alt">Ctrl + Alt</option>
            </select>
          </Setting>
          <div className="disk-estimate">
            <HardDrive aria-hidden="true" />
            <span>Estimated {estimatedGbHour.toFixed(1)} GB per hour</span>
          </div>
        </aside>
      </section>
      <footer className="capture-footer">
        <div role="status">
          {notice || 'Ctrl+Shift+R start · Ctrl+Shift+P pause · Ctrl+Shift+S stop'}
        </div>
        <button
          className="record-action"
          disabled={!canStart}
          onClick={() => void beginCountdown()}
        >
          <span /> Start recording
        </button>
      </footer>
    </main>
  );
}

function Setting({
  icon,
  label,
  children,
}: {
  icon: React.ReactNode;
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="setting-row">
      <span className="setting-icon">{icon}</span>
      <label>{label}</label>
      <div>{children}</div>
    </div>
  );
}
