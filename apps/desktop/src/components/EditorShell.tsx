import { useEffect, useState } from 'react';
import { getActiveProject, getEditorState, listRecentProjects } from '@/ipc/client';
import type { EditorState, ProjectSummary } from '@/ipc/types';

// Recordly editor-shell skeleton (top bar, left rail, canvas, right
// inspector, bottom timeline). Canvas/timeline become functional in Phase 2;
// AI panels stay disabled until their explicit phase.
const RAIL = ['Media', 'Background', 'Annotations', 'Captions', 'Audio', 'AI', 'MCP'] as const;

function railTitle(item: (typeof RAIL)[number]): string {
  if (item === 'AI') return 'AI walkthrough begins in a later phase (Phase 4)';
  if (item === 'MCP') return 'MCP integrations begin in a later phase';
  return item;
}

export function EditorShell() {
  const [project, setProject] = useState<ProjectSummary | null>(null);
  const [section, setSection] = useState<(typeof RAIL)[number]>('Media');
  const [editor, setEditor] = useState<EditorState | null>(null);

  useEffect(() => {
    void (async () => {
      const active = await getActiveProject().catch(() => '');
      const recents = await listRecentProjects().catch(() => []);
      const current = recents.find((item) => item.path === active) ?? recents[0] ?? null;
      setProject(current);
      if (current) setEditor(await getEditorState(current.path).catch(() => null));
    })();
  }, []);

  const aiDisabled = section === 'AI' || section === 'MCP';

  return (
    <main className="editor-shell launch-theme" aria-label="Kiri editor">
      <header className="editor-topbar" data-tauri-drag-region>
        <strong>Kiri Editor</strong>
        <span title={project?.path ?? ''}>{project ? project.title : 'No project open'}</span>
      </header>
      <div className="editor-body">
        <nav className="editor-rail" aria-label="Editor sections">
          {RAIL.map((item) => {
            const disabled = item === 'AI' || item === 'MCP';
            return (
              <button
                key={item}
                aria-pressed={section === item}
                disabled={disabled}
                title={railTitle(item)}
                onClick={() => {
                  if (!disabled) setSection(item);
                }}
              >
                <span>{item}</span>
                {disabled && (
                  <span className="later-badge" aria-hidden="true">
                    Later
                  </span>
                )}
              </button>
            );
          })}
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
        <span>
          {editor
            ? `Timeline: ${editor.zooms.length} zooms · ${editor.clips.length} clips · ${editor.captions.length} captions`
            : 'Timeline: trim, split, zoom regions, and captions arrive in Phase 2.'}
        </span>
      </footer>
    </main>
  );
}
