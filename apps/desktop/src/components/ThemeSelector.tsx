import { Monitor, Moon, Sun } from 'lucide-react';
import { useTheme, type ThemeMode } from '@/theme/ThemeProvider';

const options: { value: ThemeMode; label: string; icon: typeof Monitor }[] = [
  { value: 'system', label: 'System', icon: Monitor },
  { value: 'light', label: 'Light', icon: Sun },
  { value: 'dark', label: 'Dark', icon: Moon },
];

export function ThemeSelector() {
  const { mode, setMode } = useTheme();
  return (
    <div className="theme-selector" role="radiogroup" aria-label="Appearance">
      {options.map(({ value, label, icon: Icon }, index) => (
        <button
          key={value}
          type="button"
          role="radio"
          aria-checked={mode === value}
          tabIndex={mode === value ? 0 : -1}
          className={mode === value ? 'is-active' : ''}
          onClick={() => setMode(value)}
          onKeyDown={(event) => {
            const direction =
              event.key === 'ArrowRight' || event.key === 'ArrowDown'
                ? 1
                : event.key === 'ArrowLeft' || event.key === 'ArrowUp'
                  ? -1
                  : 0;
            if (!direction) return;
            event.preventDefault();
            const next = options[(index + direction + options.length) % options.length];
            if (!next) return;
            setMode(next.value);
            const buttons =
              event.currentTarget.parentElement?.querySelectorAll<HTMLButtonElement>(
                '[role="radio"]',
              );
            buttons?.[(index + direction + options.length) % options.length]?.focus();
          }}
          title={`${label} theme`}
        >
          <Icon size={14} aria-hidden="true" />
          <span className="sr-only">{label}</span>
        </button>
      ))}
    </div>
  );
}
