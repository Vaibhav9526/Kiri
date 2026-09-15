import type { EditorState } from '@/ipc/types';

export type TimelineRegionKind = 'zoom' | 'clip' | 'trim' | 'speed' | 'caption';

export interface TimelineSelection {
  kind: TimelineRegionKind;
  id: string;
}

export interface TimelineCounts {
  zooms: number;
  clips: number;
  trims: number;
  speeds: number;
  captions: number;
}

export function trackCounts(editor: EditorState | null | undefined): TimelineCounts {
  if (!editor) return { zooms: 0, clips: 0, trims: 0, speeds: 0, captions: 0 };
  return {
    zooms: editor.zooms.length,
    clips: editor.clips.length,
    trims: editor.trims.length,
    speeds: editor.speeds.length,
    captions: editor.captions.length,
  };
}

export function timelineSummaryText(editor: EditorState | null | undefined): string {
  if (!editor) return 'Timeline: trim, split, zoom regions, and captions arrive in Phase 2.';
  const counts = trackCounts(editor);
  return `Timeline: ${counts.zooms} zooms · ${counts.clips} clips · ${counts.captions} captions`;
}

export function getTimelineDurationMs(editor: EditorState | null | undefined): number {
  if (!editor) return 10_000;
  let max = 0;
  for (const list of [editor.zooms, editor.clips, editor.trims, editor.speeds, editor.captions]) {
    for (const region of list) {
      if (Number.isFinite(region.endMs) && region.endMs > max) max = region.endMs;
    }
  }
  // Keep a usable axis even for empty projects; clamp to a sane upper bound
  // so a corrupt large timestamp cannot collapse the layout math.
  if (!Number.isFinite(max) || max <= 0) return 10_000;
  return Math.min(Math.max(Math.round(max), 1_000), 86_400_000);
}

export function formatTimeMs(ms: number): string {
  if (!Number.isFinite(ms) || ms < 0) ms = 0;
  const rounded = Math.round(ms);
  const minutes = Math.floor(rounded / 60_000);
  const seconds = Math.floor((rounded % 60_000) / 1000);
  const millis = rounded % 1000;
  const sec = String(seconds).padStart(2, '0');
  const milli = String(millis).padStart(3, '0');
  return `${minutes}:${sec}.${milli}`;
}

export function blockStyle(
  startMs: number,
  endMs: number,
  durationMs: number,
): { left: string; width: string } {
  const safeDuration = Number.isFinite(durationMs) && durationMs > 0 ? durationMs : 1;
  const safeStart = Number.isFinite(startMs) ? Math.max(0, startMs) : 0;
  const safeEnd = Number.isFinite(endMs) ? Math.max(safeStart, endMs) : safeStart;
  const left = Math.min(100, Math.max(0, (safeStart / safeDuration) * 100));
  const rawWidth = ((safeEnd - safeStart) / safeDuration) * 100;
  // Keep zero-length regions visible so they remain selectable.
  const width = Math.min(100 - left, Math.max(rawWidth, 0.75));
  return { left: `${left.toFixed(3)}%`, width: `${width.toFixed(3)}%` };
}

const ASPECT_RATIO_CSS: Record<string, string> = {
  '16:9': '16 / 9',
  '9:16': '9 / 16',
  '1:1': '1 / 1',
  '4:3': '4 / 3',
};

export function aspectRatioToCss(value: string | null | undefined): string | undefined {
  if (value == null) return undefined;
  const trimmed = value.trim();
  if (trimmed === '' || trimmed.toLowerCase() === 'source') return undefined;
  if (ASPECT_RATIO_CSS[trimmed]) return ASPECT_RATIO_CSS[trimmed];
  // Accept already-CSS forms like "16 / 9".
  const normalized = trimmed.replace(/\s+/g, ' ');
  const compact = normalized.replace(/\s/g, '');
  if (ASPECT_RATIO_CSS[compact]) return ASPECT_RATIO_CSS[compact];
  if (/^\d+(\.\d+)?\s*\/\s*\d+(\.\d+)?$/.test(normalized)) return normalized;
  return undefined;
}

export type FoundRegion =
  | { kind: 'zoom'; region: EditorState['zooms'][number] }
  | { kind: 'clip'; region: EditorState['clips'][number] }
  | { kind: 'trim'; region: EditorState['trims'][number] }
  | { kind: 'speed'; region: EditorState['speeds'][number] }
  | { kind: 'caption'; region: EditorState['captions'][number] };

export function findRegion(
  editor: EditorState | null | undefined,
  selection: TimelineSelection | null | undefined,
): FoundRegion | null {
  if (!editor || !selection) return null;
  switch (selection.kind) {
    case 'zoom': {
      const region = editor.zooms.find((item) => item.id === selection.id);
      return region ? { kind: 'zoom', region } : null;
    }
    case 'clip': {
      const region = editor.clips.find((item) => item.id === selection.id);
      return region ? { kind: 'clip', region } : null;
    }
    case 'trim': {
      const region = editor.trims.find((item) => item.id === selection.id);
      return region ? { kind: 'trim', region } : null;
    }
    case 'speed': {
      const region = editor.speeds.find((item) => item.id === selection.id);
      return region ? { kind: 'speed', region } : null;
    }
    case 'caption': {
      const region = editor.captions.find((item) => item.id === selection.id);
      return region ? { kind: 'caption', region } : null;
    }
    default:
      return null;
  }
}

export function regionDurationMs(startMs: number, endMs: number): number {
  if (!Number.isFinite(startMs) || !Number.isFinite(endMs)) return 0;
  return Math.max(0, Math.round(endMs - startMs));
}
