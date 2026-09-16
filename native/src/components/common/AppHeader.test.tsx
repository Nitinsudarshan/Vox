import React from 'react';
import { describe, expect, it, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { TooltipProvider } from '@/components/ui/tooltip';
import { AppHeader } from './AppHeader';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn().mockResolvedValue(undefined),
}));

vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: vi.fn().mockReturnValue({
    minimize: vi.fn().mockResolvedValue(undefined),
    hide: vi.fn().mockResolvedValue(undefined),
    close: vi.fn().mockResolvedValue(undefined),
    onCloseRequested: vi.fn().mockResolvedValue(() => {}),
  }),
}));

describe('AppHeader', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  const defaultProps = {
    sidebarOpen: false,
    onToggleSidebar: vi.fn(),
    activeTab: 'home' as const,
    onNavigateHome: vi.fn(),
    onCloseClick: vi.fn(),
  };

  const renderHeader = (props = {}) => {
    return {
      user: userEvent.setup(),
      ...render(
        <TooltipProvider>
          <AppHeader {...defaultProps} {...props} />
        </TooltipProvider>
      ),
    };
  };

  it('renders unified header with data-tauri-drag-region', () => {
    const { container } = renderHeader();

    const header = container.querySelector('header');
    expect(header).toBeInTheDocument();
    expect(header).toHaveAttribute('data-tauri-drag-region');
  });

  it('handles sidebar toggle clicking', async () => {
    const onToggleSidebar = vi.fn();
    const { user } = renderHeader({ onToggleSidebar, sidebarOpen: false });

    const menuBtn = screen.getByRole('button', { name: 'Toggle Sidebar Navigation' });
    await user.click(menuBtn);

    expect(onToggleSidebar).toHaveBeenCalledTimes(1);
  });

  it('shows VOX home breadcrumb on home tab', () => {
    renderHeader({ activeTab: 'home' });

    expect(screen.getByRole('button', { name: 'Navigate to Vox Home' })).toBeInTheDocument();
    expect(screen.queryByText('Meetings')).not.toBeInTheDocument();
  });

  it('shows section breadcrumb on non-home tab and navigates home on logo click', async () => {
    const onNavigateHome = vi.fn();
    const { user } = renderHeader({ activeTab: 'meetings', onNavigateHome });

    expect(screen.getByText('Meetings')).toBeInTheDocument();

    const homeBtn = screen.getByRole('button', { name: 'Navigate to Vox Home' });
    await user.click(homeBtn);

    expect(onNavigateHome).toHaveBeenCalledTimes(1);
  });

  it('renders theme toggle and window controls in header', async () => {
    const onCloseClick = vi.fn();
    const { user } = renderHeader({ onCloseClick });

    // Theme toggle
    expect(screen.getByRole('button', { name: /switch to (dark|light) mode/i })).toBeInTheDocument();

    // Window controls
    expect(screen.getByRole('button', { name: 'Minimize' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Hide Vox' })).toBeInTheDocument();

    const closeBtn = screen.getByRole('button', { name: 'Close Vox' });
    await user.click(closeBtn);

    expect(onCloseClick).toHaveBeenCalledTimes(1);
  });
});
