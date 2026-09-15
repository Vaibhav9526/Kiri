import { render, screen, fireEvent } from '@testing-library/react';
import { vi } from 'vitest';
import { EditorShell } from './EditorShell';
import { getActiveProject, getEditorState, listRecentProjects } from '@/ipc/client';

vi.mock('@/ipc/client', () => ({
  getActiveProject: vi.fn(),
  getEditorState: vi.fn(),
  listRecentProjects: vi.fn(),
}));

const mockedActive = vi.mocked(getActiveProject);
const mockedRecents = vi.mocked(listRecentProjects);
const mockedEditor = vi.mocked(getEditorState);

const project = {
  id: 'bd9cde5e-d889-460e-98bf-8075fc93b220',
  title: 'Demo Walkthrough',
  path: 'D:/videos/demo.kiri',
  updatedAt: '2026-01-01T00:00:00Z',
  missing: false,
};

const editor = {
  version: 2,
  appearance: {
    background: '#0f1115',
    padding: 48,
    borderRadius: 12,
    shadow: 0.35,
    aspectRatio: null,
  },
  zooms: [
    {
      id: 'z1',
      startMs: 100,
      endMs: 900,
      depth: 2,
      focus: { cx: 0.5, cy: 0.5 },
      mode: 'manual' as const,
    },
  ],
  clips: [{ id: 'c1', startMs: 0, endMs: 1000, speed: 1, muted: false }],
  trims: [],
  speeds: [],
  captions: [{ id: 'cap1', startMs: 0, endMs: 500, text: 'Hello', words: [] }],
  webcam: {
    enabled: false,
    sourcePath: null,
    timeOffsetMs: 0,
    mirror: true,
    crop: { x: 0, y: 0, width: 1, height: 1 },
    positionX: 0.85,
    positionY: 0.85,
    size: 0.25,
    reactToZoom: true,
    roundness: 0.2,
    shadow: 0.4,
  },
};

describe('EditorShell', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockedActive.mockResolvedValue(project.path);
    mockedRecents.mockResolvedValue([project]);
    mockedEditor.mockResolvedValue(editor);
  });

  it('loads the active project and summarizes the timeline', async () => {
    render(<EditorShell />);
    expect(screen.getByText('No project open')).toBeInTheDocument();
    expect(await screen.findByText(/Demo Walkthrough/)).toBeInTheDocument();
    expect(await screen.findByText('Timeline: 1 zooms · 1 clips · 1 captions')).toBeInTheDocument();
  });

  it('keeps AI surfaces disabled with honest copy', async () => {
    render(<EditorShell />);
    await screen.findByText(/Demo Walkthrough/);
    const ai = screen.getByRole('button', { name: /AI/ });
    const mcp = screen.getByRole('button', { name: /MCP/ });
    expect(ai).toBeDisabled();
    expect(mcp).toBeDisabled();
    expect(ai.getAttribute('title')).toContain('AI walkthrough begins in a later phase');

    // Disabled rails never become the active inspector section.
    fireEvent.click(ai);
    expect(screen.getByRole('heading', { name: 'Media' })).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: /Media/ }));
    expect(screen.getByText('Inspector controls for Media arrive in Phase 2.')).toBeInTheDocument();
  });

  it('switches inspector sections without touching disabled rails', async () => {
    render(<EditorShell />);
    await screen.findByText(/Demo Walkthrough/);
    fireEvent.click(screen.getByRole('button', { name: /Captions/ }));
    expect(screen.getByRole('heading', { name: 'Captions' })).toBeInTheDocument();
    expect(
      screen.getByText('Inspector controls for Captions arrive in Phase 2.'),
    ).toBeInTheDocument();
  });

  it('falls back to placeholder copy when no editor state is available', async () => {
    mockedEditor.mockRejectedValue(new Error('no editor'));
    render(<EditorShell />);
    await screen.findByText(/Demo Walkthrough/);
    expect(
      await screen.findByText(
        'Timeline: trim, split, zoom regions, and captions arrive in Phase 2.',
      ),
    ).toBeInTheDocument();
  });

  it('shows no-project copy when recents are empty', async () => {
    mockedActive.mockResolvedValue('');
    mockedRecents.mockResolvedValue([]);
    mockedEditor.mockClear();
    render(<EditorShell />);
    expect(
      await screen.findByText(
        'Timeline: trim, split, zoom regions, and captions arrive in Phase 2.',
      ),
    ).toBeInTheDocument();
    expect(screen.getAllByText('No project open').length).toBeGreaterThan(0);
    expect(mockedEditor).not.toHaveBeenCalled();
  });
});
