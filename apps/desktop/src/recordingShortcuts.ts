import { WebviewWindow } from '@tauri-apps/api/webviewWindow';
import { register, unregisterAll } from '@tauri-apps/plugin-global-shortcut';
import { getRecordingStatus, pauseRecording, resumeRecording, stopRecording } from '@/ipc/client';
async function applyShortcuts() {
  await unregisterAll();
  const modifier =
    localStorage.getItem('kiri.shortcutProfile') === 'ctrl-alt' ? 'Ctrl+Alt' : 'Ctrl+Shift';
  await register(`${modifier}+R`, () => {
    void (async () => {
      const selector = await WebviewWindow.getByLabel('source-selector');
      await selector?.show();
      await selector?.setFocus();
    })();
  });
  await register(`${modifier}+P`, () => {
    void (async () => {
      const status = await getRecordingStatus();
      if (!status) return;
      if (status.state === 'paused') await resumeRecording();
      else await pauseRecording();
    })();
  });
  await register(`${modifier}+S`, () => {
    void (async () => {
      if (await getRecordingStatus()) await stopRecording();
    })();
  });
}
export async function registerRecordingShortcuts() {
  if (!('__TAURI_INTERNALS__' in window)) return () => undefined;
  await applyShortcuts();
  const update = () => void applyShortcuts();
  window.addEventListener('kiri-shortcuts-changed', update);
  return () => {
    window.removeEventListener('kiri-shortcuts-changed', update);
    void unregisterAll();
  };
}
