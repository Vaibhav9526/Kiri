import { render, screen } from '@testing-library/react';
import { vi } from 'vitest';
import { Home } from './Home';
import { ThemeProvider } from '@/theme/ThemeProvider';

vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn() }));
vi.mock('@/ipc/client', () => ({
  listRecentProjects: vi.fn().mockResolvedValue([]),
  listRecoverableRecordings: vi.fn().mockResolvedValue([]),
  recoverRecording: vi.fn(),
  createProject: vi.fn(),
  openProject: vi.fn(),
  setActiveProject: vi.fn().mockResolvedValue(undefined),
}));
describe('Home', () => {
  it('renders the launch hierarchy and honest phase availability', async () => {
    render(
      <ThemeProvider>
        <Home />
      </ThemeProvider>,
    );
    expect(screen.getByRole('heading', { name: 'Create a walkthrough' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /New Kiri Project/ })).toBeEnabled();
    expect(screen.getByRole('button', { name: /New Manual Recording/ })).toBeEnabled();
    expect(screen.getByRole('button', { name: /New AI Walkthrough/ })).toHaveTextContent(
      /unavailable until Phase 4/i,
    );
    expect(await screen.findByText('No local projects yet')).toBeInTheDocument();
  });
});
