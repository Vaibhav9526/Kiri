import { getCurrentWindow } from '@tauri-apps/api/window';
import { Home } from '@/components/Home';
import { RecordingController } from '@/components/RecordingController';
import { SourceSelector } from '@/components/SourceSelector';
import { useEffect } from 'react';
import { registerRecordingShortcuts } from '@/recordingShortcuts';

function windowLabel() {
  try {
    return getCurrentWindow().label;
  } catch {
    return new URLSearchParams(location.search).get('window') ?? 'main';
  }
}
export function App() {
  useEffect(() => {
    let cleanup: () => void = () => undefined;
    void registerRecordingShortcuts().then((value) => {
      cleanup = value;
    });
    return () => cleanup();
  }, []);
  switch (windowLabel()) {
    case 'source-selector':
      return <SourceSelector />;
    case 'recording-controller':
      return <RecordingController />;
    default:
      return <Home />;
  }
}
