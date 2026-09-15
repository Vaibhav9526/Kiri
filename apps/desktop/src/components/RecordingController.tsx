import { Loader2, Mic, Pause, Play, Square } from 'lucide-react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { WebviewWindow } from '@tauri-apps/api/webviewWindow';
import { useEffect, useState } from 'react';
import { getRecordingStatus, pauseRecording, resumeRecording, stopRecording } from '@/ipc/client';
import type { RecordingStatus } from '@/ipc/types';

function formatTime(micros: number) {
  const seconds = Math.floor(micros / 1_000_000);
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  return [hours, minutes, seconds % 60].map((value) => String(value).padStart(2, '0')).join(':');
}

export function RecordingController() {
  const [status, setStatus] = useState<RecordingStatus | null>(null);
  const [notice, setNotice] = useState('');
  const [busy, setBusy] = useState(false);
  const [stopping, setStopping] = useState(false);

  useEffect(() => {
    let mounted = true;
    async function refresh() {
      try {
        const value = await getRecordingStatus();
        if (mounted) {
          setStatus(value);
          if (value) setNotice('');
        }
      } catch (error) {
        if (mounted) setNotice(String(error));
      }
    }
    void refresh();
    const timer = window.setInterval(() => void refresh(), 500);
    return () => {
      mounted = false;
      window.clearInterval(timer);
    };
  }, []);

  const paused = status?.state === 'paused';
  const idle = !status || status.state === 'stopped';

  async function togglePause() {
    if (!status || idle) return;
    setBusy(true);
    try {
      setStatus(paused ? await resumeRecording() : await pauseRecording());
      setNotice('');
    } catch (error) {
      setNotice(String(error));
    } finally {
      setBusy(false);
    }
  }

  async function stop() {
    if (!status || idle) return;
    setBusy(true);
    setStopping(true);
    try {
      const result = await stopRecording();
      setStatus(result.status);
      try {
        localStorage.setItem('kiri.lastCaptureDiagnostics', JSON.stringify(result.diagnostics));
      } catch {
        // Diagnostics cache is best-effort; recording output is already final.
      }
      const main = await WebviewWindow.getByLabel('main');
      await main?.show();
      await main?.setFocus();
      await getCurrentWindow().hide();
    } catch (error) {
      setNotice(String(error));
    } finally {
      setBusy(false);
      setStopping(false);
    }
  }

  if (stopping) {
    return (
      <main className="controller-shell is-recording kiri-hud-bar" aria-label="Finishing recording">
        <Loader2 className="spin" aria-hidden="true" />
        <span className="finalizing-row">
          <span>
            Preparing recording
            <small>Opening the editor in a moment</small>
          </span>
        </span>
      </main>
    );
  }

  return (
    <main
      className={
        idle
          ? 'controller-shell is-idle kiri-hud-bar'
          : paused
            ? 'controller-shell is-paused kiri-hud-bar'
            : 'controller-shell is-recording kiri-hud-bar'
      }
      aria-label="Recording controller"
      data-tauri-drag-region
    >
      <div className="record-state">
        <span className="record-dot" />
        <strong>{idle ? 'IDLE' : paused ? 'PAUSED' : 'REC'}</strong>
      </div>
      <span className="timer" aria-label="Elapsed recording time">
        {formatTime(status?.elapsedMicros ?? 0)}
      </span>
      <span className="controller-divider" aria-hidden="true" />
      {idle ? (
        <span className="controller-empty" role="status">
          No active recording
        </span>
      ) : (
        <>
          <span title="Microphone cannot be toggled while recording">
            <button type="button" disabled aria-label="Microphone cannot be toggled while recording">
              <Mic aria-hidden="true" />
            </button>
          </span>
          <span className="controller-divider" aria-hidden="true" />
          <button
            type="button"
            disabled={busy || !status}
            aria-label={paused ? 'Resume recording' : 'Pause recording'}
            title={paused ? 'Resume recording' : 'Pause recording'}
            className={paused ? 'is-green' : ''}
            onClick={() => void togglePause()}
          >
            {paused ? <Play aria-hidden="true" /> : <Pause aria-hidden="true" />}
          </button>
          <button
            type="button"
            className="stop-button"
            disabled={busy || !status}
            aria-label="Stop recording"
            title="Stop recording"
            onClick={() => void stop()}
          >
            <Square aria-hidden="true" />
          </button>
        </>
      )}
      {notice && (
        <span className="controller-status" role="status" title={notice}>
          {notice}
        </span>
      )}
    </main>
  );
}
