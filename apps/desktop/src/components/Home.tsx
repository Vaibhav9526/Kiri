import { FolderOpen, MonitorUp, MousePointer2, Plus, Sparkles, Video, XCircle } from 'lucide-react';
import { open } from '@tauri-apps/plugin-dialog';
import { WebviewWindow } from '@tauri-apps/api/webviewWindow';
import { useEffect, useState, type FormEvent } from 'react';
import {
  createProject,
  listRecentProjects,
  listRecoverableRecordings,
  openProject,
  recoverRecording,
  setActiveProject,
} from '@/ipc/client';
import type { ProjectSummary, RecoveryCandidate } from '@/ipc/types';
import { ThemeSelector } from './ThemeSelector';

export function Home() {
  const [recents, setRecents] = useState<ProjectSummary[]>([]);
  const [recoveries, setRecoveries] = useState<RecoveryCandidate[]>([]);
  const [notice, setNotice] = useState('');
  const [busy, setBusy] = useState(false);
  const [isNaming, setIsNaming] = useState(false);
  const [projectName, setProjectName] = useState('Untitled Walkthrough');
  const [createIntent, setCreateIntent] = useState<'project' | 'recording'>('project');
  useEffect(() => {
    void listRecentProjects()
      .then(setRecents)
      .catch((error: unknown) => setNotice(String(error)));
    void listRecoverableRecordings()
      .then(setRecoveries)
      .catch((error: unknown) => setNotice(String(error)));
  }, []);

  async function chooseCreate(event: FormEvent) {
    event.preventDefault();
    const title = projectName.trim();
    if (!title) {
      setNotice('Enter a project name before choosing a location.');
      return;
    }
    const parent = await open({
      directory: true,
      multiple: false,
      title: 'Choose where to create the Kiri project',
    });
    if (!parent || Array.isArray(parent)) return;
    setBusy(true);
    try {
      const item = await createProject(parent, title);
      setRecents((current) => [item, ...current.filter((value) => value.id !== item.id)]);
      setIsNaming(false);
      const warnings: string[] = [];
      try {
        await setActiveProject(item.path);
      } catch (error) {
        warnings.push(`capture setup may not see it yet (${String(error)})`);
      }
      if (createIntent === 'recording') {
        try {
          const selector = await WebviewWindow.getByLabel('source-selector');
          await selector?.show();
          await selector?.setFocus();
        } catch (error) {
          warnings.push(`capture window did not open (${String(error)})`);
        }
      }
      setNotice(
        warnings.length ? `Created ${item.title}. Warning: ${warnings.join('; ')}` : `Created ${item.title}`,
      );
    } catch (error) {
      setNotice(`Project was not created. ${String(error)}`);
    } finally {
      setBusy(false);
    }
  }

  async function openRecent(path: string, missing: boolean) {
    if (missing) {
      setNotice('This project moved or is unavailable. Locate it with Open Kiri Project.');
      return;
    }
    setBusy(true);
    try {
      const item = await openProject(path);
      setRecents((current) => [item, ...current.filter((value) => value.id !== item.id)]);
      try {
        await setActiveProject(item.path);
      } catch {
        // Local fallback keeps capture working; backend sync is best-effort here.
      }
      setNotice(`Opened ${item.title}`);
    } catch (error) {
      setNotice(`Project could not be opened. ${String(error)}`);
    } finally {
      setBusy(false);
    }
  }

  async function chooseOpen() {
    const path = await open({ directory: true, multiple: false, title: 'Open a .kiri project' });
    if (!path || Array.isArray(path)) return;
    await openRecent(path, false);
  }

  async function recover(item: RecoveryCandidate) {
    setBusy(true);
    try {
      const project = await recoverRecording(item.projectPath);
      setRecoveries((current) => current.filter((value) => value.sessionId !== item.sessionId));
      setRecents((current) => [project, ...current.filter((value) => value.id !== project.id)]);
      try {
        await setActiveProject(project.path);
      } catch {
        // Local fallback keeps capture working; backend sync is best-effort here.
      }
      setNotice(
        `Recovered ${item.finalizedSegments} finalized segments from ${item.projectTitle}.`,
      );
    } catch (error: unknown) {
      setNotice(`Recovery failed. ${String(error)}`);
    } finally {
      setBusy(false);
    }
  }

  return (
    <main className="launch-shell">
      <header className="titlebar" data-tauri-drag-region>
        <div className="brand">
          <img src="/kiri-logo.png" alt="" />
          <span>Kiri</span>
          <span className="phase-badge">Foundation</span>
        </div>
        <ThemeSelector />
      </header>
      <section className="launch-content" aria-labelledby="home-heading">
        <div className="intro">
          <h1 id="home-heading">Create a walkthrough</h1>
          <p>Start with a local project. Your media and edits will stay on this PC.</p>
        </div>
        {recoveries.length > 0 && (
          <section className="recovery-offer" aria-labelledby="recovery-heading">
            <div>
              <strong id="recovery-heading">Interrupted recording found</strong>
              <small>Finalized segments can be restored without modifying captured media.</small>
            </div>
            {recoveries.map((item) => (
              <button
                key={item.sessionId}
                disabled={busy}
                onClick={() => void recover(item)}
              >
                Recover
              </button>
            ))}
          </section>
        )}
        <div className="primary-actions">
          <button
            className="action action-primary"
            onClick={() => {
              setCreateIntent('project');
              setIsNaming(true);
            }}
            disabled={busy}
          >
            <span className="action-icon">
              <Plus aria-hidden="true" />
            </span>
            <span>
              <strong>New Kiri Project</strong>
              <small>Create the portable project foundation.</small>
            </span>
          </button>
          <button
            className="action"
            onClick={() => {
              setCreateIntent('recording');
              setProjectName('Untitled Recording');
              setIsNaming(true);
            }}
            disabled={busy}
          >
            <span className="action-icon">
              <MousePointer2 aria-hidden="true" />
            </span>
            <span>
              <strong>New Manual Recording</strong>
              <small>Choose a source and record locally.</small>
            </span>
          </button>
          <button className="action" disabled title="AI Walkthrough begins in Phase 4 — later phase">
            <span className="action-icon">
              <Sparkles aria-hidden="true" />
            </span>
            <span>
              <strong>New AI Walkthrough</strong>
              <small>Later phase · unavailable until Phase 4</small>
            </span>
          </button>
        </div>
        {isNaming && (
          <form className="project-create" onSubmit={(event) => void chooseCreate(event)}>
            <label htmlFor="project-name">Project name</label>
            <input
              id="project-name"
              value={projectName}
              onChange={(event) => setProjectName(event.target.value)}
              autoFocus
              maxLength={120}
            />
            <button type="button" onClick={() => setIsNaming(false)}>
              Cancel
            </button>
            <button type="submit" disabled={busy}>
              {busy ? 'Creating…' : 'Choose location'}
            </button>
          </form>
        )}
        <div className="secondary-actions">
          <button disabled title="Recording import begins in Phase 2 — later phase">
            <Video aria-hidden="true" />
            Open Recording <span>Later phase · Phase 2</span>
          </button>
          <button onClick={() => void chooseOpen()} disabled={busy}>
            <FolderOpen aria-hidden="true" />
            Open Kiri Project
          </button>
        </div>
        {notice && (
          <div className="notice" role="status">
            <MonitorUp aria-hidden="true" />
            {notice}
            <button aria-label="Dismiss message" onClick={() => setNotice('')}>
              <XCircle aria-hidden="true" />
            </button>
          </div>
        )}
        <section className="recent" aria-labelledby="recent-heading">
          <div className="section-heading">
            <h2 id="recent-heading">Recent projects</h2>
            <span>{recents.length}</span>
          </div>
          {recents.length === 0 ? (
            <div className="empty-state">
              <FolderOpen aria-hidden="true" />
              <div>
                <strong>No local projects yet</strong>
                <p>Create a project or open an existing `.kiri` directory.</p>
              </div>
            </div>
          ) : (
            <ul>
              {recents.map((item) => (
                <li key={item.id}>
                  <button
                    disabled={busy}
                    onClick={() => void openRecent(item.path, item.missing)}
                    title={item.missing ? 'Project moved or unavailable' : `Open ${item.title}`}
                  >
                    <span className="project-glyph">K</span>
                    <span>
                      <strong>{item.title}</strong>
                      <small>{item.path}</small>
                    </span>
                    {item.missing && <em>Missing</em>}
                  </button>
                </li>
              ))}
            </ul>
          )}
        </section>
      </section>
    </main>
  );
}
