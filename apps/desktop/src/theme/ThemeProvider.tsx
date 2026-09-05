import { emit, listen } from '@tauri-apps/api/event';
/* eslint-disable react-refresh/only-export-components */
import { createContext, useContext, useEffect, useState, type ReactNode } from 'react';

export type ThemeMode = 'system' | 'light' | 'dark';
const key = 'kiri.theme.mode';
const media = () => window.matchMedia('(prefers-color-scheme: dark)');
const resolve = (mode: ThemeMode) =>
  mode === 'system' ? (media().matches ? 'dark' : 'light') : mode;
const ThemeContext = createContext<{ mode: ThemeMode; setMode: (mode: ThemeMode) => void } | null>(
  null,
);

export function ThemeProvider({ children }: { children: ReactNode }) {
  const [mode, updateMode] = useState<ThemeMode>(
    () => (localStorage.getItem(key) as ThemeMode | null) ?? 'system',
  );
  const apply = (next: ThemeMode) => {
    document.documentElement.dataset.themeMode = next;
    document.documentElement.dataset.theme = resolve(next);
  };
  useEffect(() => {
    apply(mode);
    const query = media();
    const onSystem = () => {
      if (mode === 'system') apply(mode);
    };
    query.addEventListener('change', onSystem);
    let unlisten = () => {};
    if ('__TAURI_INTERNALS__' in window)
      void listen<ThemeMode>('kiri://theme-changed', ({ payload }) => {
        if (payload !== mode) {
          localStorage.setItem(key, payload);
          updateMode(payload);
        }
      }).then((fn) => {
        unlisten = fn;
      });
    return () => {
      query.removeEventListener('change', onSystem);
      unlisten();
    };
  }, [mode]);
  const setMode = (next: ThemeMode) => {
    localStorage.setItem(key, next);
    updateMode(next);
    apply(next);
    if ('__TAURI_INTERNALS__' in window) void emit('kiri://theme-changed', next);
  };
  const value = { mode, setMode };
  return <ThemeContext.Provider value={value}>{children}</ThemeContext.Provider>;
}
export function useTheme() {
  const value = useContext(ThemeContext);
  if (!value) throw new Error('useTheme must be inside ThemeProvider');
  return value;
}
