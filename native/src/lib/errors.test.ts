import { describe, expect, it } from 'vitest';
import { describeError } from './errors';

describe('describeError', () => {
  it('reads a Tauri CommandError', () => {
    expect(describeError({ code: 'VAULT_BUSY', message: 'Stop the recording first.' })).toBe(
      'Stop the recording first.',
    );
  });

  it('reads an Error and a plain string', () => {
    expect(describeError(new Error('boom'))).toBe('boom');
    expect(describeError('the command said no')).toBe('the command said no');
  });

  it('falls back when there is nothing readable', () => {
    expect(describeError(undefined, 'Could not save.')).toBe('Could not save.');
    expect(describeError({ code: 'X' }, 'Could not save.')).toBe('Could not save.');
    expect(describeError('   ', 'Could not save.')).toBe('Could not save.');
  });
});
