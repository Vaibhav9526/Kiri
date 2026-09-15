import { useEffect, useState } from 'react';
import { getRecordingStatus } from '@/ipc/client';
import type { RecordingStatus } from '@/ipc/types';

function formatTime(micros: number) {
  const seconds = Math.floor(micros / 1_000_000);
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  return [hours, minutes, seconds % 60].map((value) => String(value).padStart(2, '0')).join(':');
}

// Transparent click-through HUD overlay, mirroring Recordly's hud-overlay
// window language (pill bar, blinking REC dot, amber paused state).
// Recording controls stay in the controller window; this surface only
// reflects live status.
export function HudOverlay() {
  const [status, setStatus] = useState<RecordingStatus | null>(null);

  useEffect(() => {
    let mounted = true;
    const refresh = () => {
      void getRecordingStatus()
        .then((value) => {
          if (mounted) setStatus(value);
        })
        .catch(() => undefined);
    };
    refresh();
    const timer = window.setInterval(refresh, 500);
    return () => {
      mounted = false;
      window.clearInterval(timer);
    };
  }, []);

  const state = status?.state ?? 'idle';
  const label = state === 'paused' ? 'PAUSED' : state === 'recording' ? 'REC' : 'Kiri';
  const pillClass =
    state === 'paused' ? 'hud-pill is-paused' : state === 'recording' ? 'hud-pill is-recording' : 'hud-pill';
  const elapsed = status && state !== 'stopped' ? formatTime(status.elapsedMicros) : null;

  return (
    <main className="hud-overlay" aria-label="Recording HUD">
      <span
        className={pillClass}
        data-tauri-drag-region
        title={elapsed ? `${label} · ${elapsed}` : (status?.message ?? 'Kiri recording status')}
      >
        <span className="hud-dot" aria-hidden="true" />
        {label}
      </span>
    </main>
  );
}
