import { invoke } from '@tauri-apps/api/core';
import { projectSummarySchema, type ProjectSummary } from './types';

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
