import React from 'react';
import { describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { CloseConfirmationDialog } from './CloseConfirmationDialog';

describe('CloseConfirmationDialog', () => {
  it('renders confirmation prompt and actions when open', () => {
    render(
      <CloseConfirmationDialog
        open={true}
        onOpenChange={vi.fn()}
        onConfirmClose={vi.fn()}
      />
    );

    expect(screen.getByText('Close Vox?')).toBeInTheDocument();
    expect(screen.getByText(/Are you sure you want to close Vox\?/i)).toBeInTheDocument();
    expect(screen.getByText(/To keep Vox running silently in the background, use/i)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Cancel' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Close Vox' })).toBeInTheDocument();
  });

  it('calls onOpenChange(false) when Cancel is clicked', async () => {
    const onOpenChange = vi.fn();
    const onConfirmClose = vi.fn();
    const user = userEvent.setup();

    render(
      <CloseConfirmationDialog
        open={true}
        onOpenChange={onOpenChange}
        onConfirmClose={onConfirmClose}
      />
    );

    await user.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(onOpenChange).toHaveBeenCalledWith(false);
    expect(onConfirmClose).not.toHaveBeenCalled();
  });

  it('calls onConfirmClose when Close Vox is clicked', async () => {
    const onOpenChange = vi.fn();
    const onConfirmClose = vi.fn();
    const user = userEvent.setup();

    render(
      <CloseConfirmationDialog
        open={true}
        onOpenChange={onOpenChange}
        onConfirmClose={onConfirmClose}
      />
    );

    await user.click(screen.getByRole('button', { name: 'Close Vox' }));
    expect(onConfirmClose).toHaveBeenCalledTimes(1);
  });
});
