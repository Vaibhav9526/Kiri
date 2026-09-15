import {
  allSettingsSchema,
  audioDeviceSchema,
  audioMeterSchema,
  cameraDeviceSchema,
  captionCueSchema,
  captureDiagnosticsSchema,
  captureSourceSchema,
  clipRegionSchema,
  editorStateSchema,
  jobProgressSchema,
  projectSummarySchema,
  recoveryCandidateSchema,
  recordingStatusSchema,
  stopRecordingResultSchema,
  zoomRegionSchema,
} from './types';

const validEditorState = {
  version: 2,
  appearance: {
    background: '#0f1115',
    padding: 48,
    borderRadius: 12,
    shadow: 0.35,
    aspectRatio: null,
  },
  zooms: [],
  clips: [],
  trims: [],
  speeds: [],
  captions: [],
  webcam: {
    enabled: false,
    sourcePath: null,
    timeOffsetMs: 0,
    mirror: true,
    crop: { x: 0, y: 0, width: 1, height: 1 },
    positionX: 0.85,
    positionY: 0.85,
    size: 0.25,
    reactToZoom: true,
    roundness: 0.2,
    shadow: 0.4,
  },
};

const validSettings = {
  app: { theme: 'dark', reduceMotionFollowsSystem: true, recordingsDir: null },
  recording: { microphoneId: null, systemAudio: true, cameraId: null, fps: 60 },
  countdown: { seconds: 3 },
  hudOverlay: { visible: true, clickEffects: false },
  shortcuts: 'ctrl-shift',
};

describe('ipc zod schemas', () => {
  it('validates project summaries and rejects empty titles', () => {
    const valid = {
      id: 'bd9cde5e-d889-460e-98bf-8075fc93b220',
      title: 'Demo',
      path: 'D:/videos/demo.kiri',
      updatedAt: '2026-01-01T00:00:00Z',
      missing: false,
    };
    expect(projectSummarySchema.parse(valid)).toEqual(valid);
    expect(projectSummarySchema.safeParse({ ...valid, id: 'nope' }).success).toBe(false);
    expect(projectSummarySchema.safeParse({ ...valid, title: '' }).success).toBe(false);
  });

  it('validates capture sources with availability states', () => {
    const valid = {
      id: 'display:0',
      kind: 'display',
      title: 'Primary display',
      processName: null,
      bounds: { left: 0, top: 0, width: 1920, height: 1080 },
      dpi: 120,
      availability: 'available',
      thumbnailDataUrl: null,
    };
    expect(captureSourceSchema.parse(valid)).toEqual(valid);
    expect(captureSourceSchema.safeParse({ ...valid, availability: 'gone' }).success).toBe(false);
    expect(captureSourceSchema.safeParse({ ...valid, dpi: 0 }).success).toBe(false);
  });

  it('validates audio devices, meters, and camera devices', () => {
    expect(
      audioDeviceSchema.parse({
        id: 'mic:1',
        name: 'Microphone',
        kind: 'microphone',
        isDefault: true,
      }),
    ).toBeDefined();
    expect(audioMeterSchema.parse({ peak: 0.5, clipped: false })).toEqual({
      peak: 0.5,
      clipped: false,
    });
    expect(audioMeterSchema.safeParse({ peak: 2, clipped: false }).success).toBe(false);
    expect(
      cameraDeviceSchema.parse({
        id: '0',
        name: 'Webcam',
        formats: [{ width: 1280, height: 720, fps: 30 }],
      }),
    ).toBeDefined();
  });

  it('validates recording status, diagnostics, and stop results', () => {
    const status = {
      state: 'recording',
      elapsedMicros: 1000,
      segmentIndex: 1,
      message: 'ok',
    };
    expect(recordingStatusSchema.parse(status)).toEqual(status);
    expect(recordingStatusSchema.safeParse({ ...status, state: 'done' }).success).toBe(false);
    expect(recordingStatusSchema.safeParse({ ...status, elapsedMicros: -1 }).success).toBe(false);

    const diagnostics = {
      sourceFps: 60,
      encodedFrames: 10,
      droppedFrames: 0,
      queueDepth: 0,
      encoder: 'Media Foundation H.264',
      gpuPath: true,
      audioDriftMillis: 0,
      deviceEvents: [],
    };
    expect(captureDiagnosticsSchema.parse(diagnostics)).toEqual(diagnostics);
    expect(stopRecordingResultSchema.parse({ status, diagnostics })).toBeDefined();
  });

  it('validates job progress and recovery candidates', () => {
    expect(
      jobProgressSchema.parse({
        jobId: 'bd9cde5e-d889-460e-98bf-8075fc93b220',
        state: 'running',
        completed: 2,
        total: 10,
        message: 'working',
        updatedAt: '2026-01-01T00:00:00Z',
      }),
    ).toBeDefined();
    expect(
      recoveryCandidateSchema.parse({
        projectPath: 'D:/videos/demo.kiri',
        projectTitle: 'Demo',
        sessionId: 'bd9cde5e-d889-460e-98bf-8075fc93b220',
        finalizedSegments: 2,
      }),
    ).toBeDefined();
    expect(
      recoveryCandidateSchema.safeParse({
        projectPath: 'x',
        projectTitle: 'y',
        sessionId: 'nope',
        finalizedSegments: 1,
      }).success,
    ).toBe(false);
  });

  it('applies editor defaults for zoom mode and caption words', () => {
    const zoom = zoomRegionSchema.parse({
      id: 'z1',
      startMs: 0,
      endMs: 500,
      depth: 2,
      focus: { cx: 0.5, cy: 0.5 },
    });
    expect(zoom.mode).toBe('manual');

    const caption = captionCueSchema.parse({
      id: 'c1',
      startMs: 0,
      endMs: 500,
      text: 'Hello',
    });
    expect(caption.words).toEqual([]);
  });

  it('rejects out-of-range zoom depth and clip speed', () => {
    const baseZoom = {
      id: 'z1',
      startMs: 0,
      endMs: 500,
      depth: 2,
      focus: { cx: 0.5, cy: 0.5 },
      mode: 'manual',
    };
    expect(zoomRegionSchema.safeParse({ ...baseZoom, depth: 0 }).success).toBe(false);
    expect(zoomRegionSchema.safeParse({ ...baseZoom, depth: 7 }).success).toBe(false);

    const baseClip = { id: 'c1', startMs: 0, endMs: 500, speed: 1, muted: false };
    expect(clipRegionSchema.safeParse({ ...baseClip, speed: 0.1 }).success).toBe(false);
    expect(clipRegionSchema.safeParse({ ...baseClip, speed: 8 }).success).toBe(false);
    expect(clipRegionSchema.parse(baseClip).speed).toBe(1);
  });

  it('validates a full editor state document', () => {
    expect(editorStateSchema.parse(validEditorState)).toEqual(validEditorState);
    expect(editorStateSchema.safeParse({ ...validEditorState, version: 'two' }).success).toBe(
      false,
    );
  });

  it('validates all-settings documents and rejects unknown themes', () => {
    expect(allSettingsSchema.parse(validSettings)).toEqual(validSettings);
    expect(
      allSettingsSchema.safeParse({
        ...validSettings,
        app: { ...validSettings.app, theme: 'neon' },
      }).success,
    ).toBe(false);
    expect(
      allSettingsSchema.safeParse({
        ...validSettings,
        shortcuts: 'cmd-shift',
      }).success,
    ).toBe(false);
  });
});
