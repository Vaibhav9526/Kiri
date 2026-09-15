import { Mic, Pause, Play, Square } from 'lucide-react';
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

  useEffect(() => {
    let mounted = true;
    async function refresh() {
      try {
        const value = await getRecordingStatus();
        if (mounted && value) setStatus(value);
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

  async function togglePause() {
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
    setBusy(true);
    try {
      const result = await stopRecording();
      setStatus(result.status);
      localStorage.setItem('kiri.lastCaptureDiagnostics', JSON.stringify(result.diagnostics));
      const main = await WebviewWindow.getByLabel('main');
      await main?.show();
      await main?.setFocus();
      await getCurrentWindow().hide();
    } catch (error) {
      setNotice(String(error));
    } finally {
      setBusy(false);
    }
  }

  return (
    <main
      className={paused ? 'controller-shell is-paused' : 'controller-shell is-recording'}
      aria-label="Recording controller"
      title={notice || status?.message}
    >
      <div className="record-state">
        <span className="record-dot" />
        <strong>{paused ? 'PAUSED' : 'REC'}</strong>
      </div>
      <span className="timer">{formatTime(status?.elapsedMicros ?? 0)}</span>
      <span className="controller-divider" />
      <button
        disabled={busy || !status}
        aria-label={paused ? 'Resume recording' : 'Pause recording'}
        onClick={() => void togglePause()}
      >
        {paused ? <Play /> : <Pause />}
      </button>
      <button
        className="stop-button"
        disabled={busy || !status}
        aria-label="Stop recording"
        onClick={() => void stop()}
      >
        <Square />
      </button>
      <span className="mic-state" title="Microphone source is recorded independently">
        <Mic />
      </span>
    </main>
  );
}
