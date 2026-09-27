import { test } from 'node:test';
import assert from 'node:assert/strict';
import { parseRustCommands, checkFrontendSource } from './verify-invoke-contract.mjs';

const rust = `
#[tauri::command]
pub async fn rename_meeting(
    state: State<'_, AppState>,
    meeting_id: String,
    title: Option<String>,
) -> Result<(), CommandError> { todo!() }

#[tauri::command]
pub fn get_settings(app: AppHandle) -> Result<(), CommandError> { todo!() }
`;

test('parses parameters and drops the ones Tauri injects', () => {
  const commands = parseRustCommands(rust);
  assert.deepEqual(Object.keys(commands).sort(), ['get_settings', 'rename_meeting']);
  assert.deepEqual(commands.rename_meeting.params, { meeting_id: 'String', title: 'Option<String>' });
  assert.deepEqual(commands.get_settings.params, {});
});

test('accepts a matching call in camelCase or snake_case', () => {
  const commands = parseRustCommands(rust);
  assert.deepEqual(checkFrontendSource("invoke('rename_meeting', { meetingId: id })", 'a.ts', commands), []);
  assert.deepEqual(checkFrontendSource("invoke('rename_meeting', { meeting_id: id, title })", 'a.ts', commands), []);
  assert.deepEqual(checkFrontendSource("invoke('get_settings')", 'a.ts', commands), []);
});

test('reports unknown commands, unknown keys and missing required arguments', () => {
  const commands = parseRustCommands(rust);
  const [unknown] = checkFrontendSource("invoke('move_to_trash', { id })", 'a.ts', commands);
  assert.match(unknown, /move_to_trash/);
  const wrongKey = checkFrontendSource("invoke('rename_meeting', { request: { meetingId } })", 'b.ts', commands);
  assert.equal(wrongKey.length, 2);
  assert.match(wrongKey[0], /passes 'request'/);
  assert.match(wrongKey[1], /without its required 'meetingId'/);
});

test('a call passing a variable is checked for its name only', () => {
  const commands = parseRustCommands(rust);
  assert.deepEqual(checkFrontendSource("invoke('rename_meeting', args)", 'a.ts', commands), []);
});
