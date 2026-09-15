import { invoke } from '@tauri-apps/api/core';
import {
  allSettingsSchema,
  audioDeviceSchema,
  audioMeterSchema,
  cameraDeviceSchema,
  captureSourceSchema,
  editorStateSchema,
  projectSummarySchema,
  recoveryCandidateSchema,
  type AllSettings,
  type AudioDevice,
  type AudioMeter,
  type CameraDevice,
  type CaptureSource,
  type EditorState,
  recordingStatusSchema,
  stopRecordingResultSchema,
  type RecordingStatus,
  type StartRecordingRequest,
  type StopRecordingResult,
  type ProjectSummary,
  type RecoveryCandidate,
} from './types';

const isTauri = () => '__TAURI_INTERNALS__' in window;
export async function listRecentProjects(): Promise<ProjectSummary[]> {
  if (!isTauri()) return [];
  return projectSummarySchema.array().parse(await invoke('list_recent_projects'));
}
export async function createProject(parent: string, title: string): Promise<ProjectSummary> {
  return projectSummarySchema.parse(await invoke('create_project', { request: { parent, title } }));
}
export async function openProject(path: string): Promise<ProjectSummary> {
  return projectSummarySchema.parse(await invoke('open_project', { request: { path } }));
}

export async function listCaptureSources(): Promise<CaptureSource[]> {
  if (!isTauri()) return [];
  return captureSourceSchema.array().parse(await invoke('list_capture_sources'));
}
export async function getCaptureThumbnail(sourceId: string): Promise<string | null> {
  if (!isTauri()) return null;
  return invoke<string>('capture_source_thumbnail', { sourceId });
}
export async function listAudioDevices(): Promise<AudioDevice[]> {
  if (!isTauri()) return [];
  return audioDeviceSchema.array().parse(await invoke('list_audio_devices'));
}
export async function getAudioMeter(deviceId: string | null): Promise<AudioMeter> {
  if (!isTauri()) return { peak: 0, clipped: false };
  return audioMeterSchema.parse(await invoke('audio_meter', { kind: 'microphone', deviceId }));
}
export async function listCameraDevices(): Promise<CameraDevice[]> {
  if (!isTauri()) return [];
  return cameraDeviceSchema.array().parse(await invoke('list_camera_devices'));
}
export async function startRecording(request: StartRecordingRequest): Promise<RecordingStatus> {
  return recordingStatusSchema.parse(await invoke('start_recording', { request }));
}
export async function pauseRecording(): Promise<RecordingStatus> {
  return recordingStatusSchema.parse(await invoke('pause_recording'));
}
export async function resumeRecording(): Promise<RecordingStatus> {
  return recordingStatusSchema.parse(await invoke('resume_recording'));
}
export async function stopRecording(): Promise<StopRecordingResult> {
  return stopRecordingResultSchema.parse(await invoke('stop_recording'));
}
export async function getRecordingStatus(): Promise<RecordingStatus | null> {
  if (!isTauri()) return null;
  const value = await invoke('recording_status');
  return value === null ? null : recordingStatusSchema.parse(value);
}
export async function listRecoverableRecordings(): Promise<RecoveryCandidate[]> {
  if (!isTauri()) return [];
  return recoveryCandidateSchema.array().parse(await invoke('list_recoverable_recordings'));
}
export async function recoverRecording(projectPath: string): Promise<ProjectSummary> {
  return projectSummarySchema.parse(await invoke('recover_recording', { projectPath }));
}
// Backend-held session state shared across windows (Recordly keeps the
// selected source/project in the main process; webviews share no storage).
export async function setActiveProject(path: string): Promise<void> {
  if (!isTauri()) {
    localStorage.setItem('kiri.captureProjectPath', path);
    return;
  }
  await invoke('set_active_project', { path });
  localStorage.setItem('kiri.captureProjectPath', path);
}
export async function getActiveProject(): Promise<string> {
  const fallback = localStorage.getItem('kiri.captureProjectPath') ?? '';
  if (!isTauri()) return fallback;
  const value = await invoke<string | null>('get_active_project');
  if (value) {
    localStorage.setItem('kiri.captureProjectPath', value);
    return value;
  }
  return fallback;
}
export async function setSelectedSource(sourceId: string): Promise<void> {
  if (!isTauri()) {
    localStorage.setItem('kiri.selectedSource', sourceId);
    return;
  }
  await invoke('set_selected_source', { sourceId });
  localStorage.setItem('kiri.selectedSource', sourceId);
}
export async function getSelectedSource(): Promise<string> {
  const fallback = localStorage.getItem('kiri.selectedSource') ?? '';
  if (!isTauri()) return fallback;
  const value = await invoke<string | null>('get_selected_source');
  return value ?? fallback;
}
export async function getPlatform(): Promise<string> {
  if (!isTauri()) return 'browser';
  return invoke<string>('get_platform');
}
export async function getAllSettings(): Promise<AllSettings | null> {
  if (!isTauri()) return null;
  return allSettingsSchema.parse(await invoke('get_all_settings'));
}
export async function saveAllSettings(settings: AllSettings): Promise<void> {
  if (!isTauri()) return;
  await invoke('save_all_settings', { settings });
}
export async function getEditorState(projectPath: string): Promise<EditorState | null> {
  if (!isTauri()) return null;
  return editorStateSchema.parse(await invoke('get_editor_state', { projectPath }));
}
export async function saveEditorState(
  projectPath: string,
  editor: EditorState,
): Promise<void> {
  if (!isTauri()) return;
  await invoke('save_editor_state', { projectPath, editor });
}
