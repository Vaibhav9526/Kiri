import { vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}));

const mockedInvoke = vi.mocked(invoke);

function setTauri(enabled: boolean) {
  const target = window as unknown as Record<string, unknown>;
  if (enabled) {
    Object.defineProperty(window, '__TAURI_INTERNALS__', {
      configurable: true,
      value: {},
    });
  } else {
    delete target['__TAURI_INTERNALS__'];
  }
}

describe('ipc client', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
    setTauri(false);
  });

  afterEach(() => {
    setTauri(false);
  });

  it('returns browser fallbacks without invoking Tauri', async () => {
    const client = await import('./client');
    expect(await client.listRecentProjects()).toEqual([]);
    expect(await client.listCaptureSources()).toEqual([]);
    expect(await client.listAudioDevices()).toEqual([]);
    expect(await client.listCameraDevices()).toEqual([]);
    expect(await client.listRecoverableRecordings()).toEqual([]);
    expect(await client.getRecordingStatus()).toBeNull();
    expect(await client.getAudioMeter('mic:1')).toEqual({ peak: 0, clipped: false });
    expect(await client.getCaptureThumbnail('display:0')).toBeNull();
    expect(await client.getPlatform()).toBe('browser');
    expect(await client.getAllSettings()).toBeNull();
    expect(await client.getEditorState('D:/videos/demo.kiri')).toBeNull();
    expect(mockedInvoke).not.toHaveBeenCalled();
  });

  it('persists session state to localStorage when Tauri is absent', async () => {
    const client = await import('./client');
    await client.setActiveProject('D:/videos/demo.kiri');
    expect(localStorage.getItem('kiri.captureProjectPath')).toBe('D:/videos/demo.kiri');
    expect(await client.getActiveProject()).toBe('D:/videos/demo.kiri');
    await client.setSelectedSource('display:0');
    expect(localStorage.getItem('kiri.selectedSource')).toBe('display:0');
    expect(await client.getSelectedSource()).toBe('display:0');
    expect(mockedInvoke).not.toHaveBeenCalled();
  });

  it('parses Tauri project lists through zod', async () => {
    setTauri(true);
    const client = await import('./client');
    mockedInvoke.mockResolvedValue([
      {
        id: 'bd9cde5e-d889-460e-98bf-8075fc93b220',
        title: 'Demo',
        path: 'D:/videos/demo.kiri',
        updatedAt: '2026-01-01T00:00:00Z',
        missing: false,
      },
    ]);
    const recents = await client.listRecentProjects();
    expect(recents).toHaveLength(1);
    expect(mockedInvoke).toHaveBeenCalledWith('list_recent_projects');
  });

  it('rejects malformed Tauri payloads instead of passing them through', async () => {
    setTauri(true);
    const client = await import('./client');
    mockedInvoke.mockResolvedValue([{ id: 'nope', title: '', path: 42 }]);
    await expect(client.listRecentProjects()).rejects.toThrow();
  });

  it('returns null recording status only for explicit null payloads', async () => {
    setTauri(true);
    const client = await import('./client');
    mockedInvoke.mockResolvedValue(null);
    expect(await client.getRecordingStatus()).toBeNull();
    mockedInvoke.mockResolvedValue({
      state: 'paused',
      elapsedMicros: 5,
      segmentIndex: 1,
      message: 'paused',
    });
    expect(await client.getRecordingStatus()).toMatchObject({ state: 'paused' });
  });

  it('reads the Tauri-held active project and caches it locally', async () => {
    setTauri(true);
    const client = await import('./client');
    mockedInvoke.mockResolvedValue('D:/videos/tauri.kiri');
    expect(await client.getActiveProject()).toBe('D:/videos/tauri.kiri');
    expect(localStorage.getItem('kiri.captureProjectPath')).toBe('D:/videos/tauri.kiri');
  });
});
