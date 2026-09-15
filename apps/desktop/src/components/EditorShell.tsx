import { useEffect, useState } from 'react';
import { getActiveProject, listRecentProjects } from '@/ipc/client';
import type { ProjectSummary } from '@/ipc/types';

// Phase 0 redo: Recordly editor-shell skeleton (top bar, left rail, canvas,
// right inspector, bottom timeline). Canvas/timeline become functional in
// Phase 2; AI panels stay disabled until their explicit phase.
const RAIL = ['Media', 'Background', 'Annotations', 'Captions', 'Audio', 'AI', 'MCP'] as const;

export function EditorShell() {
  const [project, setProject] = useState<ProjectSummary | null>(null);
  const [section, setSection] = useState<(typeof RAIL)[number]>('Media');

  useEffect(() => {
    void (async () => {
      const active = await getActiveProject().catch(() => '');
      const recents = await listRecentProjects().catch(() => []);
      setProject(recents.find((item) => item.path === active) ?? recents[0] ?? null);
    })();
  }, []);

  const aiDisabled = section === 'AI' || section === 'MCP';

  return (
    <main className="editor-shell" aria-label="Kiri editor">
      <header className="editor-topbar" data-tauri-drag-region>
        <strong>Kiri Editor</strong>
        <span>{project ? project.title : 'No project open'}</span>
      </header>
      <div className="editor-body">
        <nav className="editor-rail" aria-label="Editor sections">
          {RAIL.map((item) => (
            <button
              key={item}
              aria-pressed={section === item}
              disabled={item === 'AI' || item === 'MCP'}
              title={
                item === 'AI'
                  ? 'AI walkthrough begins in a later phase'
                  : item === 'MCP'
                    ? 'MCP integrations begin in a later phase'
                    : item
              }
              onClick={() => setSection(item)}
            >
              {item}
            </button>
          ))}
        </nav>
        <section className="editor-canvas" aria-label="Canvas preview">
          <div className="canvas-placeholder">
            <p>Canvas preview arrives in Phase 2.</p>
            <small>Preview and exported frames must match for golden scenes.</small>
          </div>
        </section>
        <aside className="editor-inspector" aria-label="Inspector">
          <h2>{section}</h2>
          {aiDisabled ? (
            <p>Unavailable until its explicit phase. Core editing ships first.</p>
          ) : (
            <p>Inspector controls for {section} arrive in Phase 2.</p>
          )}
        </aside>
      </div>
      <footer className="editor-timeline" aria-label="Timeline">
        <span>Timeline: trim, split, zoom regions, and captions arrive in Phase 2.</span>
      </footer>
    </main>
  );
}
