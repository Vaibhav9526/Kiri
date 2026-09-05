import { fireEvent, render, screen } from '@testing-library/react';
import { ThemeProvider } from './ThemeProvider';
import { ThemeSelector } from '@/components/ThemeSelector';
describe('ThemeProvider', () => {
  it('persists only the selected mode and applies its resolved theme', () => {
    localStorage.clear();
    render(
      <ThemeProvider>
        <ThemeSelector />
      </ThemeProvider>,
    );
    fireEvent.click(screen.getByRole('radio', { name: 'Light' }));
    expect(localStorage.getItem('kiri.theme.mode')).toBe('light');
    expect(document.documentElement.dataset.theme).toBe('light');
  });
});
