import { Monitor, RectangleHorizontal, X } from 'lucide-react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { ThemeSelector } from './ThemeSelector';

export function SourceSelector() {
  return (
    <main className="floating-shell source-shell">
      <header className="floating-header" data-tauri-drag-region>
        <div>
          <strong>Choose what to record</strong>
          <span>Capture is unavailable in Phase 0</span>
        </div>
        <ThemeSelector />
        <button
          className="icon-button"
          aria-label="Close"
          onClick={() => void getCurrentWindow().close()}
        >
          <X aria-hidden="true" />
        </button>
      </header>
      <div className="source-tabs" aria-label="Future source types">
        <button disabled title="Display discovery begins in Phase 1">
          <Monitor aria-hidden="true" />
          Displays
        </button>
        <button disabled title="Window discovery begins in Phase 1">
          <RectangleHorizontal aria-hidden="true" />
          Windows
        </button>
      </div>
      <div className="source-placeholder">
        <Monitor aria-hidden="true" />
        <strong>Source discovery begins in Phase 1</strong>
        <p>This separate window boundary is ready for Windows Graphics Capture sources.</p>
      </div>
      <footer>
        <button disabled>Start recording — Phase 1</button>
      </footer>
    </main>
  );
}
