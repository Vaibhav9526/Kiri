import { render, screen } from '@testing-library/react';
import { vi } from 'vitest';
import { App } from './App';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { registerRecordingShortcuts } from '@/recordingShortcuts';

vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: vi.fn(),
}));

vi.mock('@/recordingShortcuts', () => ({
  registerRecordingShortcuts: vi.fn(),
}));

vi.mock('@/components/SourceSelector', () => ({
  SourceSelector: () => <div data-testid="window-source-selector" />,
}));
vi.mock('@/components/RecordingController', () => ({
  RecordingController: () => <div data-testid="window-recording-controller" />,
}));
vi.mock('@/components/CountdownOverlay', () => ({
  CountdownOverlay: () => <div data-testid="window-countdown" />,
}));
vi.mock('@/components/HudOverlay', () => ({
  HudOverlay: () => <div data-testid="window-hud-overlay" />,
}));
vi.mock('@/components/EditorShell', () => ({
  EditorShell: () => <div data-testid="window-editor" />,
}));
vi.mock('@/components/Home', () => ({
  Home: () => <div data-testid="window-home" />,
}));

const mockedWindow = vi.mocked(getCurrentWindow);
const mockedShortcuts = vi.mocked(registerRecordingShortcuts);

describe('App window routing', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockedShortcuts.mockResolvedValue(() => undefined);
  });

  it.each([
    ['source-selector', 'window-source-selector'],
    ['recording-controller', 'window-recording-controller'],
    ['countdown', 'window-countdown'],
    ['hud-overlay', 'window-hud-overlay'],
    ['editor', 'window-editor'],
    ['main', 'window-home'],
  ])('routes %s to the expected surface', (label, testId) => {
    mockedWindow.mockReturnValue({ label } as unknown as ReturnType<typeof getCurrentWindow>);
    render(<App />);
    expect(screen.getByTestId(testId)).toBeInTheDocument();
  });

  it('falls back to home for unknown labels', () => {
    mockedWindow.mockReturnValue({
      label: 'something-else',
    } as unknown as ReturnType<typeof getCurrentWindow>);
    render(<App />);
    expect(screen.getByTestId('window-home')).toBeInTheDocument();
  });

  it('registers recording shortcuts on mount and cleans up on unmount', () => {
    const cleanup = vi.fn();
    mockedShortcuts.mockResolvedValue(cleanup);
    mockedWindow.mockReturnValue({ label: 'main' } as unknown as ReturnType<
      typeof getCurrentWindow
    >);
    const view = render(<App />);
    expect(mockedShortcuts).toHaveBeenCalledTimes(1);
    view.unmount();
  });
});
