import React from 'react';
import { describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { NativeSidebar } from './NativeSidebar';
import type { MainTabType } from '@/types';

describe('NativeSidebar', () => {
  const defaultProps = {
    isOpen: true,
    onToggle: vi.fn(),
    activeTab: 'scribble' as MainTabType,
    setActiveTab: vi.fn(),
    account: null,
    profile: null,
    appVersion: '0.1.0',
    onOpenChangelog: vi.fn(),
    onOpenWelcome: vi.fn(),
    onOpenExplanation: vi.fn(),
  };

  it('renders primary destinations in expected order and does not include Home or capture actions', () => {
    render(<NativeSidebar {...defaultProps} />);

    // Primary items: Voice notes, Scribbles, TODOs, Knowledge Graph
    const buttons = screen.getAllByRole('button');
    const buttonLabels = buttons.map((b) => b.getAttribute('aria-label')).filter(Boolean);
    const expectedOrder = [
      'Voice Notes',
      'Scribbles',
      'TODOs',
      'Knowledge Graph',
    ];
    let lastIdx = -1;
    for (const label of expectedOrder) {
      const idx = buttonLabels.indexOf(label);
      expect(idx).toBeGreaterThan(lastIdx);
      lastIdx = idx;
    }

    // System items
    expect(screen.getByRole('button', { name: 'Tests' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Settings' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Diagnostics' })).toBeInTheDocument();

    // Must NOT have Home or capture methods as sidebar navigation items
    expect(screen.queryByRole('button', { name: 'Home' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Files' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Files & Docs' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Captures' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Web Capture' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Capture' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Typed Text' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Clipboard' })).not.toBeInTheDocument();
  });

  it('displays Local Vault under profile name and opens workspace options', async () => {
    const user = userEvent.setup();
    render(<NativeSidebar {...defaultProps} isOpen={true} />);

    expect(screen.getAllByText('Local Vault').length).toBeGreaterThan(0);

    const profileBtn = screen.getByTitle('Account & Session Settings');
    await user.click(profileBtn);

    expect(screen.getByText('Local Vault Only')).toBeInTheDocument();
    expect(screen.getByText('Hybrid Cloud Sync')).toBeInTheDocument();
  });

  it('navigates to primary and system destinations when clicked', async () => {
    const setActiveTab = vi.fn();
    const user = userEvent.setup();
    render(<NativeSidebar {...defaultProps} setActiveTab={setActiveTab} />);

    await user.click(screen.getByRole('button', { name: 'Voice Notes' }));
    expect(setActiveTab).toHaveBeenCalledWith('capture');

    await user.click(screen.getByRole('button', { name: 'Scribbles' }));
    expect(setActiveTab).toHaveBeenCalledWith('scribble');

    await user.click(screen.getByRole('button', { name: 'TODOs' }));
    expect(setActiveTab).toHaveBeenCalledWith('todos');

    await user.click(screen.getByRole('button', { name: 'Knowledge Graph' }));
    expect(setActiveTab).toHaveBeenCalledWith('graph');

    await user.click(screen.getByRole('button', { name: 'Tests' }));
    expect(setActiveTab).toHaveBeenCalledWith('tests');

    await user.click(screen.getByRole('button', { name: 'Settings' }));
    expect(setActiveTab).toHaveBeenCalledWith('settings');

    await user.click(screen.getByRole('button', { name: 'Diagnostics' }));
    expect(setActiveTab).toHaveBeenCalledWith('diagnostics');
  });
});
