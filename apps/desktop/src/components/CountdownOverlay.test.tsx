import { render, screen, fireEvent } from '@testing-library/react';
import { vi } from 'vitest';
import { CountdownOverlay } from './CountdownOverlay';

vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ hide: vi.fn() }),
}));

describe('CountdownOverlay', () => {
  it('renders the provided countdown value with live status', () => {
    render(<CountdownOverlay value={3} />);
    expect(screen.getByRole('status')).toHaveTextContent('3');
    expect(screen.getByText(/Recording starts shortly/)).toBeInTheDocument();
  });

  it('renders an ellipsis placeholder when no value is provided', () => {
    render(<CountdownOverlay />);
    expect(screen.getByRole('status')).toHaveTextContent('…');
  });

  it('renders zero without falling back to the placeholder', () => {
    render(<CountdownOverlay value={0} />);
    expect(screen.getByRole('status')).toHaveTextContent('0');
  });

  it('notifies the parent when the countdown is cancelled by click', () => {
    const onCancel = vi.fn();
    render(<CountdownOverlay value={2} onCancel={onCancel} />);
    fireEvent.click(screen.getByRole('status').firstElementChild ?? screen.getByRole('status'));
    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  it('notifies the parent when Escape is pressed', () => {
    const onCancel = vi.fn();
    render(<CountdownOverlay value={2} onCancel={onCancel} />);
    fireEvent.keyDown(window, { key: 'Escape' });
    expect(onCancel).toHaveBeenCalledTimes(1);
  });
});
