import {
  Camera,
  Check,
  Gauge,
  HardDrive,
  Home,
  Mic,
  Monitor,
  RectangleHorizontal,
  RefreshCw,
  Speaker,
  Timer,
  X,
} from 'lucide-react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { WebviewWindow } from '@tauri-apps/api/webviewWindow';
import { useEffect, useMemo, useRef, useState } from 'react';
import {
  getActiveProject,
  getAudioMeter,
  getCaptureThumbnail,
  getSelectedSource,
  listAudioDevices,
  listCameraDevices,
  listCaptureSources,
  listRecentProjects,
  setActiveProject,
  setSelectedSource,
  startRecording,
} from '@/ipc/client';
import type { AudioDevice, CameraDevice, CaptureSource, ProjectSummary } from '@/ipc/types';
import { CountdownOverlay } from './CountdownOverlay';
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

const MIC_BARS = [
  { threshold: 0.1, height: '30%' },
  { threshold: 0.25, height: '45%' },
  { threshold: 0.45, height: '60%' },
  { threshold: 0.65, height: '75%' },
  { threshold: 0.85, height: '92%' },
];

function barClass(peak: number, threshold: number, clipped: boolean) {
  if (clipped) return 'is-lit-peak';
  if (peak < threshold) return '';
  if (threshold >= 0.85) return 'is-lit-peak';
  if (threshold >= 0.65) return 'is-lit-high';
  if (threshold >= 0.45) return 'is-lit-mid';
  return 'is-lit-low';
}

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
  const cancelRef = useRef(false);
  // Backend is the source of truth for the active project/selected source;
  // localStorage is only a fallback cache for browser previews.
  const [projectPath, setProjectPath] = useState('');
  const [recents, setRecents] = useState<ProjectSummary[]>([]);

  async function refresh() {
    setLoading(true);
    setNotice('');
    try {
      const [nextSources, nextAudio, nextCameras, activePath, recentProjects] = await Promise.all([
        listCaptureSources(),
        listAudioDevices(),
        listCameraDevices(),
        getActiveProject().catch(() => ''),
        listRecentProjects().catch(() => [] as ProjectSummary[]),
      ]);
      // getSelectedSource is optional (older mocks omit it); never let a
      // missing/unavailable accessor fail the whole discovery pass.
      let savedSourceId = '';
      try {
        const accessor = getSelectedSource as unknown as
          | (() => Promise<string>)
          | undefined;
        if (typeof accessor === 'function') savedSourceId = await accessor().catch(() => '');
      } catch {
        savedSourceId = '';
      }
      const cachedPath = (() => {
        try {
          return localStorage.getItem('kiri.captureProjectPath') ?? '';
        } catch {
          return '';
        }
      })();
      const resolvedPath =
        activePath ||
        cachedPath ||
        recentProjects.find((item) => !item.missing)?.path ||
        '';
      if (resolvedPath) {
        setProjectPath(resolvedPath);
        try {
          localStorage.setItem('kiri.captureProjectPath', resolvedPath);
        } catch {
          // Cache is best-effort.
        }
      } else {
        setProjectPath('');
      }
      setRecents(recentProjects);
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
      setSourceId((current) => {
        if (current && resolved.some((value) => value.id === current)) return current;
        if (savedSourceId && resolved.some((value) => value.id === savedSourceId)) {
          const saved = resolved.find((value) => value.id === savedSourceId);
          if (saved) setTab(saved.kind);
          return savedSourceId;
        }
        return (
          resolved.find((value) => value.kind === tab && value.availability === 'available')?.id ??
          resolved.find((value) => value.availability === 'available')?.id ??
          ''
        );
      });
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
    let cancelled = false;
    const constraints: MediaStreamConstraints =
      cameraId === 'default'
        ? { video: true, audio: false }
        : { video: { deviceId: { exact: cameraId } }, audio: false };
    void navigator.mediaDevices
      .getUserMedia(constraints)
      .then((value) => {
        if (cancelled) {
          value.getTracks().forEach((track) => track.stop());
          return;
        }
        stream = value;
        if (previewRef.current) previewRef.current.srcObject = value;
      })
      .catch(() =>
        setNotice('Camera preview is unavailable. Close other camera apps and retry.'),
      );
    return () => {
      cancelled = true;
      stream?.getTracks().forEach((track) => track.stop());
    };
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
  const displayCount = useMemo(
    () => sources.filter((source) => source.kind === 'display').length,
    [sources],
  );
  const windowCount = useMemo(
    () => sources.filter((source) => source.kind === 'window').length,
    [sources],
  );
  const microphones = useMemo(
    () => audio.filter((device) => device.kind === 'microphone'),
    [audio],
  );
  const selectedSource = sources.find((source) => source.id === sourceId);
  const projectTitle = recents.find((item) => item.path === projectPath)?.title;
  const estimatedGbHour = fps === 60 ? 7.3 : 4.6;
  const canStart =
    Boolean(projectPath) &&
    selectedSource?.availability === 'available' &&
    !loading &&
    counting === null;

  function cancelCountdown() {
    cancelRef.current = true;
    setCounting(null);
    setNotice('Countdown cancelled.');
  }

  async function beginCountdown() {
    if (!canStart) {
      if (!projectPath)
        setNotice('Create a recording project from Home first, then reopen capture setup.');
      return;
    }
    cancelRef.current = false;
    if (countdown > 0) {
      for (let value = countdown; value > 0; value -= 1) {
        if (cancelRef.current) return;
        setCounting(value);
        await new Promise((resolve) => window.setTimeout(resolve, 1000));
        if (cancelRef.current) return;
      }
    }
    setCounting(0);
    try {
      await setActiveProject(projectPath);
      await setSelectedSource(sourceId);
      await startRecording({
        projectPath,
        sourceId,
        microphoneId: microphoneId || null,
        systemAudio,
        cameraId: cameraId || null,
        fps,
      });
      setCounting(null);
      const controller = await WebviewWindow.getByLabel('recording-controller');
      await controller?.show();
      await controller?.setFocus();
      await getCurrentWindow().hide();
    } catch (error) {
      setNotice('Recording did not start. ' + String(error));
      setCounting(null);
    }
  }

  async function openHome() {
    try {
      const main = await WebviewWindow.getByLabel('main');
      await main?.show();
      await main?.setFocus();
    } catch (error) {
      setNotice('Home could not be opened. ' + String(error));
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
    <main className="floating-shell capture-setup launch-theme">
      {counting !== null && <CountdownOverlay value={counting} onCancel={cancelCountdown} />}
      <header className="floating-header" data-tauri-drag-region>
        <div>
          <strong>Capture setup</strong>
          <span title={projectPath}>{projectTitle ?? projectPath ?? 'Select a recording project'}</span>
        </div>
        <ThemeSelector />
        <button
          className="icon-button"
          aria-label="Refresh devices"
          title="Refresh devices"
          onClick={() => void refresh()}
        >
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
        <button
          role="tab"
          aria-selected={tab === 'display'}
          onClick={() => selectTab('display')}
        >
          <Monitor aria-hidden="true" /> Displays
          <span aria-hidden="true" className="tab-count">
            {' '}
            · {displayCount}
          </span>
        </button>
        <button role="tab" aria-selected={tab === 'window'} onClick={() => selectTab('window')}>
          <RectangleHorizontal aria-hidden="true" /> Windows
          <span aria-hidden="true" className="tab-count">
            {' '}
            · {windowCount}
          </span>
        </button>
      </div>
      <section className="capture-body">
        <div className="source-list" role="tabpanel" aria-label={tab === 'display' ? 'Displays' : 'Windows'}>
          <div className="source-group-label">
            {tab === 'display' ? 'Displays' : 'Windows'}
            <span className={loading ? 'refreshing is-visible' : 'refreshing'}>Refreshing…</span>
          </div>
          {loading ? (
            <div className="source-loading">Discovering Windows capture sources…</div>
          ) : visibleSources.length === 0 ? (
            <div className="source-empty" role="status">
              <strong>{tab === 'display' ? 'No displays found' : 'No windows found'}</strong>
              <p>
                {tab === 'display'
                  ? 'Connect a display, then refresh.'
                  : 'Open a window (minimized windows are hidden), then refresh.'}
              </p>
              <button type="button" onClick={() => void refresh()}>
                Refresh sources
              </button>
            </div>
          ) : (
            visibleSources.map((source) => {
              const scale = Math.round((source.dpi / 96) * 100);
              const subtitle = `${source.processName ?? 'Windows display'} · ${source.bounds.width}×${source.bounds.height} · ${scale}%`;
              return (
                <button
                  key={source.id}
                  className={source.id === sourceId ? 'source-item is-selected' : 'source-item'}
                  disabled={source.availability !== 'available'}
                  onClick={() => setSourceId(source.id)}
                  title={
                    source.availability === 'available'
                      ? `${source.title} · ${subtitle}`
                      : `${source.title} is ${source.availability}`
                  }
                >
                  <span className="source-thumbnail" aria-hidden="true">
                    {source.thumbnailDataUrl ? (
                      <img
                        src={source.thumbnailDataUrl}
                        alt=""
                        onError={(event) => {
                          (event.target as HTMLImageElement).style.display = 'none';
                        }}
                      />
                    ) : source.kind === 'display' ? (
                      <Monitor />
                    ) : (
                      <RectangleHorizontal />
                    )}
                  </span>
                  <span style={{ minWidth: 0 }}>
                    <strong className="source-title" title={source.title || 'Untitled window'}>
                      {source.title || 'Untitled window'}
                    </strong>
                    <small>{subtitle}</small>
                  </span>
                  {source.id === sourceId && <Check aria-hidden="true" />}
                  {source.availability !== 'available' && <em>{source.availability}</em>}
                </button>
              );
            })
          )}
        </div>
        <aside className="capture-options" aria-label="Recording configuration">
          <Setting icon={<Mic />} label="Microphone">
            <select
              value={microphoneId}
              onChange={(event) => setMicrophoneId(event.target.value)}
              aria-label="Microphone"
            >
              <option value="">Off</option>
              {microphones.length === 0 && (
                <option value="__none" disabled className="device-empty-option">
                  No microphones found
                </option>
              )}
              {microphones.map((device) => (
                <option key={device.id} value={device.id}>
                  {device.name}
                </option>
              ))}
            </select>
          </Setting>
          <div
            className={microphoneClipped ? 'meter-row is-clipped' : 'meter-row'}
            aria-label={microphoneClipped ? 'Microphone clipping' : 'Microphone level'}
            title={microphoneClipped ? 'Microphone clipping — lower input level' : 'Microphone level'}
          >
            <span
              className="audio-bars"
              aria-hidden="true"
              style={{ flex: 1 }}
            >
              {MIC_BARS.map((bar) => (
                <span
                  key={bar.threshold}
                  className={barClass(microphonePeak, bar.threshold, microphoneClipped)}
                  style={{ height: microphonePeak >= bar.threshold ? bar.height : '15%' }}
                />
              ))}
            </span>
            <small>{microphoneId ? `${Math.round(microphonePeak * 100)}%` : 'Off'}</small>
          </div>
          <Setting icon={<Speaker />} label="System audio">
            <button
              className={systemAudio ? 'switch is-on' : 'switch'}
              role="switch"
              aria-checked={systemAudio}
              aria-label="System audio"
              onClick={() => setSystemAudio((value) => !value)}
            >
              <span />
            </button>
          </Setting>
          <Setting icon={<Camera />} label="Camera">
            <select
              value={cameraId}
              onChange={(event) => setCameraId(event.target.value)}
              aria-label="Camera"
            >
              <option value="">Off</option>
              {cameras.length === 0 && (
                <option value="__none" disabled className="device-empty-option">
                  No cameras found
                </option>
              )}
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
            <div className="segmented" role="group" aria-label="Frame rate">
              <button
                type="button"
                className={fps === 30 ? 'is-active' : ''}
                aria-pressed={fps === 30}
                onClick={() => setFps(30)}
              >
                30 fps
              </button>
              <button
                type="button"
                className={fps === 60 ? 'is-active' : ''}
                aria-pressed={fps === 60}
                onClick={() => setFps(60)}
              >
                60 fps
              </button>
            </div>
          </Setting>
          <Setting icon={<Timer />} label="Countdown">
            <select
              value={countdown}
              onChange={(event) => setCountdown(Number(event.target.value))}
              aria-label="Countdown delay"
            >
              <option value={0}>None</option>
              <option value={3}>3 seconds</option>
              <option value={5}>5 seconds</option>
              <option value={10}>10 seconds</option>
            </select>
          </Setting>
          <Setting icon={<Gauge />} label="Shortcuts">
            <select
              value={shortcutProfile}
              aria-label="Shortcut profile"
              onChange={(event) => {
                const value = event.target.value;
                setShortcutProfile(value);
                try {
                  localStorage.setItem('kiri.shortcutProfile', value);
                } catch {
                  // Preference cache is best-effort.
                }
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
          {notice ||
            (!projectPath
              ? 'Create a recording project from Home first'
              : 'Ctrl+Shift+R start · Ctrl+Shift+P pause · Ctrl+Shift+S stop')}
        </div>
        {!projectPath &&
          (recents.length > 0 ? (
            <select
              aria-label="Recording project"
              value={projectPath}
              onChange={(event) => {
                setProjectPath(event.target.value);
                try {
                  localStorage.setItem('kiri.captureProjectPath', event.target.value);
                } catch {
                  // Cache is best-effort.
                }
                void setActiveProject(event.target.value).catch(() => undefined);
              }}
            >
              <option value="">Select project…</option>
              {recents
                .filter((item) => !item.missing)
                .map((item) => (
                  <option key={item.id} value={item.path}>
                    {item.title}
                  </option>
                ))}
            </select>
          ) : (
            <button
              type="button"
              className="project-picker-row"
              onClick={() => void openHome()}
              title="Open Home to create a project"
            >
              <Home aria-hidden="true" /> Open Home
            </button>
          ))}
        <button
          className="record-action"
          disabled={!canStart}
          title={
            !projectPath
              ? 'Create a recording project from Home first'
              : !selectedSource || selectedSource.availability !== 'available'
                ? 'Select an available source'
                : 'Start recording'
          }
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
