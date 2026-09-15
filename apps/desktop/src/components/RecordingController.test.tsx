import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { vi } from 'vitest';
import { RecordingController } from './RecordingController';
import { getRecordingStatus, pauseRecording, resumeRecording, stopRecording } from '@/ipc/client';

vi.mock('@/ipc/client', () => ({
  getRecordingStatus: vi.fn(),
  pauseRecording: vi.fn(),
  resumeRecording: vi.fn(),
  stopRecording: vi.fn(),
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

import type { RecordingStatus } from '@/ipc/types';

const mockedStatus = vi.mocked(getRecordingStatus);
const mockedPause = vi.mocked(pauseRecording);
const mockedResume = vi.mocked(resumeRecording);
const mockedStop = vi.mocked(stopRecording);

function recordingStatus(overrides: Partial<RecordingStatus> = {}): RecordingStatus {
  return {
    state: 'recording',
    elapsedMicros: 3_661_000_000,
    segmentIndex: 1,
    message: 'Recording session is active',
    ...overrides,
  };
}

describe('RecordingController', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
    mockedStatus.mockResolvedValue(recordingStatus());
  });

  it('formats elapsed microseconds as HH:MM:SS and enables controls', async () => {
    render(<RecordingController />);
    expect(screen.getByText('00:00:00')).toBeInTheDocument();
    expect(await screen.findByText('01:01:01')).toBeInTheDocument();
    expect(screen.getByText('REC')).toBeInTheDocument();
    expect(await screen.findByRole('button', { name: 'Pause recording' })).toBeEnabled();
    expect(screen.getByRole('button', { name: 'Stop recording' })).toBeEnabled();
  });

  it('pauses an active recording and resumes when paused', async () => {
    mockedStatus.mockResolvedValueOnce(recordingStatus());
    mockedPause.mockResolvedValue({
      state: 'paused',
      elapsedMicros: 5_000_000,
      segmentIndex: 1,
      message: 'Recording paused; segment finalized',
    });
    render(<RecordingController />);
    const pauseButton = await screen.findByRole('button', { name: 'Pause recording' });
    fireEvent.click(pauseButton);
    await waitFor(() => expect(mockedPause).toHaveBeenCalledTimes(1));
    expect(await screen.findByText('PAUSED')).toBeInTheDocument();

    mockedResume.mockResolvedValue(recordingStatus());
    fireEvent.click(screen.getByRole('button', { name: 'Resume recording' }));
    await waitFor(() => expect(mockedResume).toHaveBeenCalledTimes(1));
    expect(await screen.findByText('REC')).toBeInTheDocument();
  });

  it('stops recording, persists diagnostics, and returns to main', async () => {
    const diagnostics = {
      sourceFps: 60,
      encodedFrames: 120,
      droppedFrames: 0,
      queueDepth: 0,
      encoder: 'Media Foundation H.264',
      gpuPath: true,
      audioDriftMillis: 0,
      deviceEvents: [] as string[],
    };
    mockedStop.mockResolvedValue({
      status: { state: 'stopped', elapsedMicros: 10_000_000, segmentIndex: 1, message: 'done' },
      diagnostics,
    });
    render(<RecordingController />);
    fireEvent.click(await screen.findByRole('button', { name: 'Stop recording' }));
    await waitFor(() => expect(mockedStop).toHaveBeenCalledTimes(1));
    expect(localStorage.getItem('kiri.lastCaptureDiagnostics')).toBe(JSON.stringify(diagnostics));
    await waitFor(() => expect(show).toHaveBeenCalledTimes(1));
    expect(setFocus).toHaveBeenCalledTimes(1);
    expect(hide).toHaveBeenCalledTimes(1);
  });

  it('surfaces pause failures without losing controller state', async () => {
    mockedPause.mockRejectedValue(new Error('pause failed'));
    render(<RecordingController />);
    fireEvent.click(await screen.findByRole('button', { name: 'Pause recording' }));
    await waitFor(() => expect(mockedPause).toHaveBeenCalledTimes(1));
    expect(screen.getByText('REC')).toBeInTheDocument();
  });

  it('renders an honest idle state with no recording controls', async () => {
    mockedStatus.mockResolvedValue(null);
    render(<RecordingController />);
    expect(await screen.findByText('IDLE')).toBeInTheDocument();
    expect(screen.getByText('No active recording')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Pause recording' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Stop recording' })).not.toBeInTheDocument();
  });
});
