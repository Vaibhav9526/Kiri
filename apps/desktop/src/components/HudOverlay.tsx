import { useEffect, useState } from 'react';
import { getRecordingStatus } from '@/ipc/client';

// Phase 0 redo: transparent click-through HUD overlay skeleton, mirroring
// Recordly's hud-overlay window. Recording controls stay in the controller
// window; this surface only reflects live status during Phase 0-1.
export function HudOverlay() {
  const [label, setLabel] = useState('Kiri');

  useEffect(() => {
    let mounted = true;
    const timer = window.setInterval(() => {
      void getRecordingStatus()
        .then((status) => {
          if (mounted && status) setLabel(status.state === 'paused' ? 'PAUSED' : 'REC');
        })
        .catch(() => undefined);
    }, 500);
    return () => {
      mounted = false;
      window.clearInterval(timer);
    };
  }, []);

  return (
    <main className="hud-overlay" aria-label="Recording HUD">
      <span className="hud-pill">{label}</span>
    </main>
  );
}
