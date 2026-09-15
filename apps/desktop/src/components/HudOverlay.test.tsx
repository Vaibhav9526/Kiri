import { render, screen, waitFor } from '@testing-library/react';
import { vi } from 'vitest';
import { HudOverlay } from './HudOverlay';
import { getRecordingStatus } from '@/ipc/client';

vi.mock('@/ipc/client', () => ({
  getRecordingStatus: vi.fn(),
}));

const mockedStatus = vi.mocked(getRecordingStatus);

function hudText() {
  return screen.getByLabelText('Recording HUD').textContent ?? '';
}

describe('HudOverlay', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockedStatus.mockResolvedValue(null);
  });

  it('renders the idle label', async () => {
    render(<HudOverlay />);
    expect(screen.getByLabelText('Recording HUD')).toBeInTheDocument();
    await waitFor(() => expect(hudText()).toContain('Kiri'));
    expect(hudText()).not.toContain('REC');
    expect(hudText()).not.toContain('PAUSED');
  });

  it('reflects a live recording state', async () => {
    mockedStatus.mockResolvedValue({
      state: 'recording',
      elapsedMicros: 65_000_000,
      segmentIndex: 1,
      message: 'Recording session is active',
    });
    render(<HudOverlay />);
    await waitFor(() => expect(hudText()).toContain('REC'));
  });

  it('reflects a paused recording state', async () => {
    mockedStatus.mockResolvedValue({
      state: 'paused',
      elapsedMicros: 2_000_000,
      segmentIndex: 1,
      message: 'Recording paused; segment finalized',
    });
    render(<HudOverlay />);
    await waitFor(() => expect(hudText()).toContain('PAUSED'));
  });

  it('keeps the idle label when status polling fails', async () => {
    mockedStatus.mockRejectedValue(new Error('ipc unavailable'));
    render(<HudOverlay />);
    await waitFor(() => expect(hudText()).toContain('Kiri'));
    expect(hudText()).not.toContain('REC');
  });
});
