import { getCurrentWindow } from '@tauri-apps/api/window';
import { useEffect, useState } from 'react';

function readInitialSeconds(explicit?: number | null): number | null {
  if (typeof explicit === 'number') return explicit;
  try {
    const params = new URLSearchParams(window.location.search);
    const raw = params.get('seconds') ?? params.get('value') ?? params.get('countdown');
    if (raw === null || raw === '') return null;
    const parsed = Number(raw);
    return Number.isFinite(parsed) && parsed >= 0 ? Math.floor(parsed) : null;
  } catch {
    return null;
  }
}

export function CountdownOverlay({
  value,
  onCancel,
}: {
  value?: number | null;
  onCancel?: () => void;
}) {
  const [seconds, setSeconds] = useState<number | null>(() => readInitialSeconds(value));
  const [isStandalone] = useState(() => value === undefined);

  useEffect(() => {
    if (value !== undefined) setSeconds(value);
  }, [value]);

  useEffect(() => {
    if (!isStandalone || value !== undefined) return;
    // Standalone countdown window (Tauri label `countdown`, 320x200): tick down
    // locally when opened with ?seconds=N so the window never idles on "…".
    if (seconds === null || seconds <= 0) return;
    const timer = window.setTimeout(() => setSeconds((current) => (current ?? 1) - 1), 1000);
    return () => window.clearTimeout(timer);
  }, [isStandalone, seconds, value]);

  async function cancel() {
    if (onCancel) {
      onCancel();
      return;
    }
    try {
      await getCurrentWindow().hide();
    } catch {
      setSeconds(null);
    }
  }

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') void cancel();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [onCancel]);

  const display = seconds ?? value ?? '…';
  const isCountdownWindow = (() => {
    try {
      return new URLSearchParams(window.location.search).get('window') === 'countdown';
    } catch {
      return false;
    }
  })();

  const hint = display === 0 ? 'Starting capture' : 'Recording starts shortly';
  if (isCountdownWindow || isStandalone) {
    return (
      <main className="countdown-window" aria-label="Recording countdown">
        <div
          className="countdown-card"
          role="status"
          aria-live="assertive"
          title="Click or press Esc to cancel"
          onClick={() => void cancel()}
          onKeyDown={(event) => {
            if (event.key === 'Escape' || event.key === 'Enter') void cancel();
          }}
          tabIndex={0}
        >
          <span>{display}</span>
          <small>{hint}</small>
        </div>
      </main>
    );
  }

  return (
    <div className="countdown-overlay" role="status" aria-live="assertive">
      <div
        className="countdown-card"
        title="Click or press Esc to cancel"
        onClick={() => void cancel()}
        onKeyDown={(event) => {
          if (event.key === 'Escape' || event.key === 'Enter') void cancel();
        }}
        tabIndex={0}
      >
        <span>{display}</span>
        <small>{hint}</small>
      </div>
    </div>
  );
}
