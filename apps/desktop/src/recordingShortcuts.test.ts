import { vi } from 'vitest';
import { WebviewWindow } from '@tauri-apps/api/webviewWindow';
import { register, unregisterAll } from '@tauri-apps/plugin-global-shortcut';
import { getRecordingStatus, pauseRecording, resumeRecording, stopRecording } from '@/ipc/client';

vi.mock('@tauri-apps/api/webviewWindow', () => ({
  WebviewWindow: { getByLabel: vi.fn() },
}));
vi.mock('@tauri-apps/plugin-global-shortcut', () => ({
  register: vi.fn(),
  unregisterAll: vi.fn(),
}));
vi.mock('@/ipc/client', () => ({
  getRecordingStatus: vi.fn(),
  pauseRecording: vi.fn(),
  resumeRecording: vi.fn(),
  stopRecording: vi.fn(),
}));

const mockedRegister = vi.mocked(register);
const mockedUnregister = vi.mocked(unregisterAll);
const mockedStatus = vi.mocked(getRecordingStatus);
// eslint-disable-next-line @typescript-eslint/unbound-method
const mockedGetByLabel = vi.mocked(WebviewWindow.getByLabel);

function setTauri(enabled: boolean) {
  if (enabled) {
    Object.defineProperty(window, '__TAURI_INTERNALS__', {
      configurable: true,
      value: {},
    });
  } else {
    delete (window as unknown as Record<string, unknown>)['__TAURI_INTERNALS__'];
  }
}

function registeredHandler(shortcut: string) {
  const call = mockedRegister.mock.calls.find(([name]) => name === shortcut);
  if (!call) throw new Error(`shortcut ${shortcut} was not registered`);
  const handler = call[1] as () => void;
  return handler;
}

describe('recordingShortcuts', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
    setTauri(true);
    mockedUnregister.mockResolvedValue(undefined);
    mockedRegister.mockResolvedValue(undefined);
  });

  afterEach(() => {
    setTauri(false);
    window.removeEventListener('kiri-shortcuts-changed', () => undefined);
  });

  it('does nothing outside Tauri', async () => {
    setTauri(false);
    const { registerRecordingShortcuts } = await import('./recordingShortcuts');
    const cleanup = await registerRecordingShortcuts();
    expect(mockedRegister).not.toHaveBeenCalled();
    expect(typeof cleanup).toBe('function');
  });

  it('registers Ctrl+Shift shortcuts by default', async () => {
    const { registerRecordingShortcuts } = await import('./recordingShortcuts');
    await registerRecordingShortcuts();
    expect(mockedUnregister).toHaveBeenCalled();
    expect(mockedRegister).toHaveBeenCalledWith('Ctrl+Shift+R', expect.any(Function));
    expect(mockedRegister).toHaveBeenCalledWith('Ctrl+Shift+P', expect.any(Function));
    expect(mockedRegister).toHaveBeenCalledWith('Ctrl+Shift+S', expect.any(Function));
  });

  it('registers Ctrl+Alt shortcuts when that profile is stored', async () => {
    localStorage.setItem('kiri.shortcutProfile', 'ctrl-alt');
    const { registerRecordingShortcuts } = await import('./recordingShortcuts');
    await registerRecordingShortcuts();
    expect(mockedRegister).toHaveBeenCalledWith('Ctrl+Alt+R', expect.any(Function));
  });

  it('toggles pause state from the P shortcut', async () => {
    const { registerRecordingShortcuts } = await import('./recordingShortcuts');
    await registerRecordingShortcuts();
    mockedStatus.mockResolvedValue({
      state: 'recording',
      elapsedMicros: 1,
      segmentIndex: 1,
      message: 'recording',
    });
    registeredHandler('Ctrl+Shift+P')();
    await vi.waitFor(() => expect(pauseRecording).toHaveBeenCalledTimes(1));

    mockedStatus.mockResolvedValue({
      state: 'paused',
      elapsedMicros: 2,
      segmentIndex: 1,
      message: 'paused',
    });
    registeredHandler('Ctrl+Shift+P')();
    await vi.waitFor(() => expect(resumeRecording).toHaveBeenCalledTimes(1));
  });

  it('shows the source selector from the R shortcut', async () => {
    const show = vi.fn();
    const setFocus = vi.fn();
    mockedGetByLabel.mockResolvedValue({ show, setFocus } as unknown as WebviewWindow);
    const { registerRecordingShortcuts } = await import('./recordingShortcuts');
    await registerRecordingShortcuts();
    registeredHandler('Ctrl+Shift+R')();
    await vi.waitFor(() => expect(mockedGetByLabel).toHaveBeenCalledWith('source-selector'));
    await vi.waitFor(() => expect(show).toHaveBeenCalledTimes(1));
  });

  it('stops only when a recording is active', async () => {
    const { registerRecordingShortcuts } = await import('./recordingShortcuts');
    await registerRecordingShortcuts();
    mockedStatus.mockResolvedValue(null);
    registeredHandler('Ctrl+Shift+S')();
    await vi.waitFor(() => expect(mockedStatus).toHaveBeenCalled());
    expect(stopRecording).not.toHaveBeenCalled();
  });
});
