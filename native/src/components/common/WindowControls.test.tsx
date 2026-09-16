import React from 'react';
import { describe, expect, it, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { invoke } from '@tauri-apps/api/core';
import { TooltipProvider } from '@/components/ui/tooltip';
import { WindowControls } from './WindowControls';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn().mockResolvedValue(undefined),
}));

vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: vi.fn().mockReturnValue({
    minimize: vi.fn().mockResolvedValue(undefined),
    hide: vi.fn().mockResolvedValue(undefined),
    close: vi.fn().mockResolvedValue(undefined),
  }),
}));

describe('WindowControls', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  const renderComponent = (onCloseClick = vi.fn()) => {
    return {
      user: userEvent.setup(),
      onCloseClick,
      ...render(
        <TooltipProvider>
          <WindowControls onCloseClick={onCloseClick} />
        </TooltipProvider>
      ),
    };
  };

  it('renders minimize, maximize, hide, and close buttons with accessible labels', () => {
    renderComponent();

    expect(screen.getByRole('button', { name: 'Minimize' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Maximize' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Hide Vox' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Close Vox' })).toBeInTheDocument();
  });

  it('triggers minimize_main_window when minimize is clicked', async () => {
    const { user } = renderComponent();

    const minimizeBtn = screen.getByRole('button', { name: 'Minimize' });
    await user.click(minimizeBtn);

    expect(vi.mocked(invoke)).toHaveBeenCalledWith('minimize_main_window');
  });

  it('triggers toggle_maximize_main_window when maximize is clicked', async () => {
    const { user } = renderComponent();

    const maximizeBtn = screen.getByRole('button', { name: 'Maximize' });
    await user.click(maximizeBtn);

    expect(vi.mocked(invoke)).toHaveBeenCalledWith('toggle_maximize_main_window');
  });

  it('triggers hide_main_window when hide is clicked', async () => {
    const { user } = renderComponent();

    const hideBtn = screen.getByRole('button', { name: 'Hide Vox' });
    await user.click(hideBtn);

    expect(vi.mocked(invoke)).toHaveBeenCalledWith('hide_main_window');
  });

  it('calls onCloseClick when close button is clicked', async () => {
    const onCloseClick = vi.fn();
    const { user } = renderComponent(onCloseClick);

    const closeBtn = screen.getByRole('button', { name: 'Close Vox' });
    await user.click(closeBtn);

    expect(onCloseClick).toHaveBeenCalledTimes(1);
    expect(vi.mocked(invoke)).not.toHaveBeenCalledWith('close_app');
  });

  it('supports keyboard navigation through all window controls', async () => {
    const onCloseClick = vi.fn();
    const { user } = renderComponent(onCloseClick);

    const minimizeBtn = screen.getByRole('button', { name: 'Minimize' });
    const maximizeBtn = screen.getByRole('button', { name: 'Maximize' });
    const hideBtn = screen.getByRole('button', { name: 'Hide Vox' });
    const closeBtn = screen.getByRole('button', { name: 'Close Vox' });

    // Focus first button
    await user.tab();
    expect(minimizeBtn).toHaveFocus();

    // Tab to maximize button
    await user.tab();
    expect(maximizeBtn).toHaveFocus();

    // Tab to hide button
    await user.tab();
    expect(hideBtn).toHaveFocus();

    // Tab to close button and press enter
    await user.tab();
    expect(closeBtn).toHaveFocus();
    await user.keyboard('{Enter}');
    expect(onCloseClick).toHaveBeenCalledTimes(1);
  });
});
