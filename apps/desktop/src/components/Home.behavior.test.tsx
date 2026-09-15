import { render, screen, fireEvent } from '@testing-library/react';
import { vi } from 'vitest';
import { Home } from './Home';
import { ThemeProvider } from '@/theme/ThemeProvider';
import {
  createProject,
  listRecentProjects,
  listRecoverableRecordings,
  openProject,
  recoverRecording,
  setActiveProject,
} from '@/ipc/client';

vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn() }));
vi.mock('@tauri-apps/api/webviewWindow', () => ({
  WebviewWindow: { getByLabel: vi.fn(() => Promise.resolve(null)) },
}));
vi.mock('@/ipc/client', () => ({
  createProject: vi.fn(),
  listRecentProjects: vi.fn(),
  listRecoverableRecordings: vi.fn(),
  openProject: vi.fn(),
  recoverRecording: vi.fn(),
  setActiveProject: vi.fn(),
}));

const mockedRecents = vi.mocked(listRecentProjects);
const mockedRecoveries = vi.mocked(listRecoverableRecordings);
const mockedRecover = vi.mocked(recoverRecording);
const mockedOpen = vi.mocked(openProject);
const mockedCreate = vi.mocked(createProject);
const mockedSetActive = vi.mocked(setActiveProject);

function renderHome() {
  return render(
    <ThemeProvider>
      <Home />
    </ThemeProvider>,
  );
}

describe('Home behavior', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockedRecents.mockResolvedValue([]);
    mockedRecoveries.mockResolvedValue([]);
    mockedCreate.mockResolvedValue({
      id: 'bd9cde5e-d889-460e-98bf-8075fc93b220',
      title: 'Demo',
      path: 'D:/videos/demo.kiri',
      updatedAt: '2026-01-01T00:00:00Z',
      missing: false,
    });
    mockedSetActive.mockResolvedValue(undefined);
  });

  it('offers recovery for interrupted recordings', async () => {
    mockedRecoveries.mockResolvedValue([
      {
        projectPath: 'D:/videos/demo.kiri',
        projectTitle: 'Demo',
        sessionId: 'bd9cde5e-d889-460e-98bf-8075fc93b220',
        finalizedSegments: 2,
      },
    ]);
    renderHome();
    expect(await screen.findByText('Interrupted recording found')).toBeInTheDocument();
    mockedRecover.mockResolvedValue({
      id: 'bd9cde5e-d889-460e-98bf-8075fc93b220',
      title: 'Demo',
      path: 'D:/videos/demo.kiri',
      updatedAt: '2026-01-01T00:00:00Z',
      missing: false,
    });
    fireEvent.click(screen.getByRole('button', { name: 'Recover' }));
    expect(await screen.findByText(/Recovered 2 finalized segments/)).toBeInTheDocument();
  });

  it('explains missing projects instead of opening them', async () => {
    mockedRecents.mockResolvedValue([
      {
        id: 'bd9cde5e-d889-460e-98bf-8075fc93b220',
        title: 'Gone',
        path: 'Z:/missing/gone.kiri',
        updatedAt: '2026-01-01T00:00:00Z',
        missing: true,
      },
    ]);
    renderHome();
    expect(await screen.findByText('Missing')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: /Gone/ }));
    expect(await screen.findByText(/This project moved or is unavailable/)).toBeInTheDocument();
    expect(mockedOpen).not.toHaveBeenCalled();
  });

  it('requires a non-empty project name before choosing a location', async () => {
    const { open } = await import('@tauri-apps/plugin-dialog');
    vi.mocked(open).mockResolvedValue('D:/videos');
    renderHome();
    await screen.findByText('No local projects yet');
    fireEvent.click(screen.getByRole('button', { name: /New Kiri Project/ }));
    const input = screen.getByLabelText('Project name');
    fireEvent.change(input, { target: { value: '   ' } });
    fireEvent.click(screen.getByRole('button', { name: 'Choose location' }));
    expect(
      await screen.findByText('Enter a project name before choosing a location.'),
    ).toBeInTheDocument();
    expect(mockedCreate).not.toHaveBeenCalled();
  });
});
