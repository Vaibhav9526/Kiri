import type { EditorState } from '@/ipc/types';
import { aspectRatioToCss } from './timelineUtils';

interface EditorCanvasProps {
  appearance: EditorState['appearance'];
  projectTitle?: string | null;
}

export function EditorCanvas({ appearance, projectTitle }: EditorCanvasProps) {
  const aspectCss = aspectRatioToCss(appearance.aspectRatio ?? null);
  return (
    <div className="editor-canvas-advanced" data-testid="editor-canvas">
      <div
        className="canvas-frame"
        data-testid="editor-canvas-frame"
        data-aspect={appearance.aspectRatio ?? 'Source'}
        style={{
          background: appearance.background,
          ...(aspectCss ? { aspectRatio: aspectCss } : {}),
        }}
        role="img"
        aria-label={`Canvas preview${projectTitle ? ` for ${projectTitle}` : ''}. Preview rendering arrives later.`}
      >
        <div className="canvas-frame-inner">
          <p>Canvas preview arrives in Phase 2.</p>
          <small>Preview and exported frames must match for golden scenes.</small>
          <small data-testid="editor-canvas-appearance">
            {appearance.aspectRatio ?? 'Source'} · {appearance.background} · pad{' '}
            {appearance.padding} · radius {appearance.borderRadius}
          </small>
          <span className="later-badge" title="Interactive preview rendering comes later">
            Preview later
          </span>
        </div>
      </div>
    </div>
  );
}
