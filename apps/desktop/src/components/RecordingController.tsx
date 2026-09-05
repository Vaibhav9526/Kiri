import { Circle, Pause, Square } from 'lucide-react';
export function RecordingController() {
  return (
    <main className="controller-shell" aria-label="Recording controller">
      <div className="record-dot" />
      <span className="timer">00:00:00</span>
      <button disabled aria-label="Pause">
        <Pause />
      </button>
      <button disabled aria-label="Stop">
        <Square />
      </button>
      <span className="phase-label">
        <Circle />
        Phase 1
      </span>
    </main>
  );
}
