import { getCurrentWindow } from '@tauri-apps/api/window';
import { Home } from '@/components/Home';
import { RecordingController } from '@/components/RecordingController';
import { SourceSelector } from '@/components/SourceSelector';

function windowLabel() {
  try {
    return getCurrentWindow().label;
  } catch {
    return new URLSearchParams(location.search).get('window') ?? 'main';
  }
}
export function App() {
  switch (windowLabel()) {
    case 'source-selector':
      return <SourceSelector />;
    case 'recording-controller':
      return <RecordingController />;
    default:
      return <Home />;
  }
}
