import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { vi } from 'vitest';
import { SourceSelector } from './SourceSelector';
import { ThemeProvider } from '@/theme/ThemeProvider';
import {
  getActiveProject,
  getAudioMeter,
  getCaptureThumbnail,
  getSelectedSource,
  listAudioDevices,
  listCameraDevices,
  listCaptureSources,
  listRecentProjects,
  setActiveProject,
  setSelectedSource,
  startRecording,
} from '@/ipc/client';
import type { CaptureSource } from '@/ipc/types';

vi.mock('@/ipc/client', () => ({
  getActiveProject: vi.fn(),
  getAudioMeter: vi.fn(),
  getCaptureThumbnail: vi.fn(),
  getSelectedSource: vi.fn(),
  listAudioDevices: vi.fn(),
  listCameraDevices: vi.fn(),
  listCaptureSources: vi.fn(),
  listRecentProjects: vi.fn(),
  setActiveProject: vi.fn(),
  setSelectedSource: vi.fn(),
  startRecording: vi.fn(),
}));

const hide = vi.fn();
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ hide }),
}));

const show = vi.fn();
const setFocus = vi.fn();
vi.mock('@tauri-apps/api/webviewWindow', () => ({
  WebviewWindow: {
    getByLabel: vi.fn(() => Promise.resolve({ show, setFocus })),
  },
}));

const mockedSources = vi.mocked(listCaptureSources);
const mockedAudio = vi.mocked(listAudioDevices);
const mockedCameras = vi.mocked(listCameraDevices);
const mockedActive = vi.mocked(getActiveProject);
const mockedSavedSource = vi.mocked(getSelectedSource);
const mockedRecents = vi.mocked(listRecentProjects);
const mockedMeter = vi.mocked(getAudioMeter);
const mockedThumbnail = vi.mocked(getCaptureThumbnail);
const mockedSetActive = vi.mocked(setActiveProject);
const mockedSetSource = vi.mocked(setSelectedSource);
const mockedStart = vi.mocked(startRecording);

const displaySource: CaptureSource = {
  id: 'display:0',
  kind: 'display',
  title: 'Primary display',
  processName: null,
  bounds: { left: 0, top: 0, width: 1920, height: 1080 },
  dpi: 96,
  availability: 'available',
  thumbnailDataUrl: null,
};

const windowSource: CaptureSource = {
  id: 'window:1',
  kind: 'window',
  title: 'Editor — Kiri',
  processName: 'kiri.exe',
  bounds: { left: 0, top: 0, width: 1440, height: 900 },
  dpi: 96,
  availability: 'available',
  thumbnailDataUrl: null,
};

function renderSelector() {
  return render(
    <ThemeProvider>
      <SourceSelector />
    </ThemeProvider>,
  );
}

describe('SourceSelector', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
    mockedSources.mockResolvedValue([displaySource, windowSource]);
    mockedAudio.mockResolvedValue([
      { id: 'mic:default', name: 'Default microphone', kind: 'microphone', isDefault: true },
    ]);
    mockedCameras.mockResolvedValue([]);
    mockedActive.mockResolvedValue('D:/videos/demo.kiri');
    mockedSavedSource.mockResolvedValue('');
    mockedRecents.mockResolvedValue([]);
    mockedMeter.mockResolvedValue({ peak: 0.5, clipped: false });
    mockedThumbnail.mockResolvedValue(null);
    mockedSetActive.mockResolvedValue(undefined);
    mockedSetSource.mockResolvedValue(undefined);
    mockedStart.mockResolvedValue({
      state: 'recording',
      elapsedMicros: 0,
      segmentIndex: 1,
      message: 'Recording started',
    });
  });

  it('discovers sources and selects the default display with estimates', async () => {
    renderSelector();
    expect(screen.getByText('Discovering Windows capture sources…')).toBeInTheDocument();
    expect(await screen.findByText('Primary display')).toBeInTheDocument();
    expect(screen.getByRole('tab', { name: /Displays/ })).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByText('Estimated 7.3 GB per hour')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Start recording/ })).toBeEnabled();
  });

  it('falls back to built-in sources when discovery returns nothing', async () => {
    mockedSources.mockResolvedValue([]);
    mockedActive.mockResolvedValue('');
    mockedSavedSource.mockResolvedValue('');
    localStorage.clear();
    renderSelector();
    expect(await screen.findByText('Primary display')).toBeInTheDocument();
    expect(
      await screen.findByText('Create a recording project from Home first'),
    ).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Open Home/ })).toBeInTheDocument();
  });

  it('switches between display and window tabs', async () => {
    renderSelector();
    await screen.findByText('Primary display');
    fireEvent.click(screen.getByRole('tab', { name: /Windows/ }));
    expect(screen.getByRole('tab', { name: /Windows/ })).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByText('Editor — Kiri')).toBeInTheDocument();
    expect(screen.queryByText('Primary display')).not.toBeInTheDocument();
  });

  it('starts recording without a countdown and focuses the controller', async () => {
    renderSelector();
    await screen.findByText('Primary display');
    fireEvent.change(screen.getByLabelText(/Countdown/), { target: { value: '0' } });
    fireEvent.click(screen.getByRole('button', { name: /Start recording/ }));
    await waitFor(() => expect(mockedSetActive).toHaveBeenCalledWith('D:/videos/demo.kiri'));
    expect(mockedSetSource).toHaveBeenCalledWith('display:0');
    expect(mockedStart).toHaveBeenCalledWith({
      projectPath: 'D:/videos/demo.kiri',
      sourceId: 'display:0',
      microphoneId: 'mic:default',
      systemAudio: true,
      cameraId: null,
      fps: 60,
    });
    await waitFor(() => expect(show).toHaveBeenCalledTimes(1));
    expect(setFocus).toHaveBeenCalledTimes(1);
    expect(hide).toHaveBeenCalledTimes(1);
  });

  it('surfaces start failures as an honest notice', async () => {
    mockedStart.mockRejectedValue(new Error('device busy'));
    renderSelector();
    await screen.findByText('Primary display');
    fireEvent.change(screen.getByLabelText(/Countdown/), { target: { value: '0' } });
    fireEvent.click(screen.getByRole('button', { name: /Start recording/ }));
    expect(await screen.findByText(/Recording did not start/)).toBeInTheDocument();
  });

  it('surfaces discovery failures and keeps fallback sources', async () => {
    mockedSources.mockRejectedValue(new Error('WGC unavailable'));
    renderSelector();
    expect(await screen.findByText(/Devices could not be refreshed/)).toBeInTheDocument();
    expect(await screen.findByText('Primary display')).toBeInTheDocument();
  });

  it('reflects microphone level and clipping state', async () => {
    mockedMeter.mockResolvedValue({ peak: 1, clipped: true });
    renderSelector();
    await screen.findByText('Primary display');
    const meter = await screen.findByLabelText('Microphone clipping');
    expect(meter).toHaveTextContent('100%');
  });

  it('restores the backend-selected source across windows', async () => {
    mockedSavedSource.mockResolvedValue('window:1');
    renderSelector();
    expect(await screen.findByText('Editor — Kiri')).toBeInTheDocument();
    expect(screen.getByRole('tab', { name: /Windows/ })).toHaveAttribute('aria-selected', 'true');
  });
});
