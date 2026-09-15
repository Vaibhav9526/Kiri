import { z } from 'zod';

export const projectSummarySchema = z.object({
  id: z.string().uuid(),
  title: z.string().min(1),
  path: z.string(),
  updatedAt: z.string(),
  missing: z.boolean(),
});
export type ProjectSummary = z.infer<typeof projectSummarySchema>;
export const jobProgressSchema = z.object({
  jobId: z.string().uuid(),
  state: z.enum(['queued', 'running', 'succeeded', 'failed', 'cancelled']),
  completed: z.number().int().nonnegative(),
  total: z.number().int().nonnegative().nullable().optional(),
  message: z.string(),
  updatedAt: z.string(),
});
export type JobProgress = z.infer<typeof jobProgressSchema>;

export const captureSourceSchema = z.object({
  id: z.string(),
  kind: z.enum(['display', 'window']),
  title: z.string(),
  processName: z.string().nullable(),
  bounds: z.object({
    left: z.number().int(),
    top: z.number().int(),
    width: z.number().int(),
    height: z.number().int(),
  }),
  dpi: z.number().int().positive(),
  availability: z.enum(['available', 'minimized', 'closed', 'protected', 'invalid']),
  thumbnailDataUrl: z.string().nullable(),
});
export type CaptureSource = z.infer<typeof captureSourceSchema>;

export const audioDeviceSchema = z.object({
  id: z.string(),
  name: z.string(),
  kind: z.enum(['microphone', 'systemLoopback']),
  isDefault: z.boolean(),
});
export type AudioDevice = z.infer<typeof audioDeviceSchema>;
export const audioMeterSchema = z.object({ peak: z.number().min(0).max(1), clipped: z.boolean() });
export type AudioMeter = z.infer<typeof audioMeterSchema>;

export const cameraDeviceSchema = z.object({
  id: z.string(),
  name: z.string(),
  formats: z.array(
    z.object({ width: z.number().int(), height: z.number().int(), fps: z.number().int() }),
  ),
});
export type CameraDevice = z.infer<typeof cameraDeviceSchema>;
export const recordingStatusSchema = z.object({
  state: z.enum(['recording', 'paused', 'stopped']),
  elapsedMicros: z.number().int().nonnegative(),
  segmentIndex: z.number().int().positive(),
  message: z.string(),
});
export type RecordingStatus = z.infer<typeof recordingStatusSchema>;

export const captureDiagnosticsSchema = z.object({
  sourceFps: z.number().nonnegative(),
  encodedFrames: z.number().int().nonnegative(),
  droppedFrames: z.number().int().nonnegative(),
  queueDepth: z.number().int().nonnegative(),
  encoder: z.string(),
  gpuPath: z.boolean(),
  audioDriftMillis: z.number(),
  deviceEvents: z.array(z.string()),
});
export const stopRecordingResultSchema = z.object({
  status: recordingStatusSchema,
  diagnostics: captureDiagnosticsSchema,
});
export type StopRecordingResult = z.infer<typeof stopRecordingResultSchema>;

export interface StartRecordingRequest {
  projectPath: string;
  sourceId: string;
  microphoneId: string | null;
  systemAudio: boolean;
  cameraId: string | null;
  fps: 30 | 60;
}
export const recoveryCandidateSchema = z.object({
  projectPath: z.string(),
  projectTitle: z.string(),
  sessionId: z.string().uuid(),
  finalizedSegments: z.number().int().nonnegative(),
});
export type RecoveryCandidate = z.infer<typeof recoveryCandidateSchema>;

// Recordly-derived editor schema (mirrors kiri-project/src/editor.rs).
// AI-generated fields are accepted but never produced locally in Phase 0-2.
export const zoomRegionSchema = z.object({
  id: z.string(),
  startMs: z.number().int(),
  endMs: z.number().int(),
  depth: z.number().int().min(1).max(6),
  focus: z.object({ cx: z.number(), cy: z.number() }),
  mode: z.enum(['auto', 'manual']).default('manual'),
});
export type ZoomRegion = z.infer<typeof zoomRegionSchema>;

export const clipRegionSchema = z.object({
  id: z.string(),
  startMs: z.number().int(),
  endMs: z.number().int(),
  speed: z.number().min(0.25).max(4),
  muted: z.boolean(),
});
export type ClipRegion = z.infer<typeof clipRegionSchema>;

export const captionCueSchema = z.object({
  id: z.string(),
  startMs: z.number().int(),
  endMs: z.number().int(),
  text: z.string(),
  words: z
    .array(z.object({ text: z.string(), startMs: z.number().int(), endMs: z.number().int() }))
    .default([]),
});
export type CaptionCue = z.infer<typeof captionCueSchema>;

export const editorStateSchema = z.object({
  version: z.number().int(),
  appearance: z.object({
    background: z.string(),
    padding: z.number(),
    borderRadius: z.number(),
    shadow: z.number(),
    aspectRatio: z.string().nullable(),
  }),
  zooms: zoomRegionSchema.array(),
  clips: clipRegionSchema.array(),
  trims: z.array(z.object({ id: z.string(), startMs: z.number().int(), endMs: z.number().int() })),
  speeds: z.array(
    z.object({
      id: z.string(),
      startMs: z.number().int(),
      endMs: z.number().int(),
      speed: z.number(),
    }),
  ),
  captions: captionCueSchema.array(),
  webcam: z.object({
    enabled: z.boolean(),
    sourcePath: z.string().nullable(),
    timeOffsetMs: z.number().int(),
    mirror: z.boolean(),
    crop: z.object({ x: z.number(), y: z.number(), width: z.number(), height: z.number() }),
    positionX: z.number(),
    positionY: z.number(),
    size: z.number(),
    reactToZoom: z.boolean(),
    roundness: z.number(),
    shadow: z.number(),
  }),
});
export type EditorState = z.infer<typeof editorStateSchema>;

export const allSettingsSchema = z.object({
  app: z.object({
    theme: z.enum(['system', 'light', 'dark']),
    reduceMotionFollowsSystem: z.boolean(),
    recordingsDir: z.string().nullable(),
  }),
  recording: z.object({
    microphoneId: z.string().nullable(),
    systemAudio: z.boolean(),
    cameraId: z.string().nullable(),
    fps: z.number().int(),
  }),
  countdown: z.object({ seconds: z.number().int() }),
  hudOverlay: z.object({ visible: z.boolean(), clickEffects: z.boolean() }),
  shortcuts: z.enum(['ctrl-shift', 'ctrl-alt']),
});
export type AllSettings = z.infer<typeof allSettingsSchema>;
