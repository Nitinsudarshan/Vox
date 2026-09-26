#!/usr/bin/env node
/**
 * Checks every `invoke('<command>', { ... })` in the native frontend against
 * the Tauri commands the Rust backend defines.
 *
 * A frontend call to a command that does not exist, or with an argument name
 * the command does not take, compiles, typechecks and passes every unit test
 * (the tests mock `invoke`), and then fails at run time as a button that does
 * nothing. Six such calls had accumulated before this check existed.
 *
 * Reported:
 * - a command the backend does not define;
 * - an argument key the command does not take (Tauri maps camelCase keys to
 *   snake_case parameters, and both spellings are accepted here);
 * - a required (non-`Option`) parameter missing from an object-literal call.
 *
 * Calls whose arguments are not an object literal — a variable passed
 * through — are checked for the command name only.
 *
 * Usage: node scripts/verify-invoke-contract.mjs   (exit 1 on any finding)
 */
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const INJECTED_TYPES = ['State<', 'AppHandle', 'Window', 'WebviewWindow'];

/** Every file under `dir` whose name passes `keep`. */
export function walk(dir, keep, out = []) {
  for (const name of readdirSync(dir)) {
    if (name === 'node_modules' || name === 'target' || name.startsWith('.')) continue;
    const path = join(dir, name);
    if (statSync(path).isDirectory()) walk(path, keep, out);
    else if (keep(name)) out.push(path);
  }
  return out;
}

/** Splits a parameter list on top-level commas. */
function splitParams(params) {
  const parts = [];
  let depth = 0;
  let current = '';
  for (const ch of params) {
    if ('<([{'.includes(ch)) depth++;
    if ('>)]}'.includes(ch)) depth--;
    if (ch === ',' && depth === 0) {
      parts.push(current);
      current = '';
    } else {
      current += ch;
    }
  }
  if (current.trim()) parts.push(current);
  return parts;
}

/** `{ name: { params: { snake_name: type } } }` for every `#[tauri::command]`. */
export function parseRustCommands(source) {
  const commands = {};
  const re =
    /#\[tauri::command[^\]]*\]\s*(?:#\[[^\]]*\]\s*)*pub(?:\(crate\))?\s+(?:async\s+)?fn\s+(\w+)\s*(?:<[^>]*>)?\s*\(([\s\S]*?)\)\s*(?:->|\{)/g;
  for (const match of source.matchAll(re)) {
    const params = {};
    for (const part of splitParams(match[2])) {
      const colon = part.indexOf(':');
      if (colon < 0) continue;
      const name = part.slice(0, colon).trim().replace(/^mut\s+/, '');
      const type = part.slice(colon + 1).trim();
      if (INJECTED_TYPES.some((t) => type.includes(t))) continue;
      params[name] = type;
    }
    commands[match[1]] = { params };
  }
  return commands;
}

const camel = (snake) => snake.replace(/_([a-z0-9])/g, (_, c) => c.toUpperCase());

/** The top-level keys of the object literal starting at `start` (just past `{`). */
function objectKeys(source, start) {
  let depth = 1;
  let topLevel = '';
  for (let i = start; i < source.length && depth > 0; i++) {
    const ch = source[i];
    if ('{[('.includes(ch)) depth++;
    else if ('}])'.includes(ch)) depth--;
    if (depth === 1) topLevel += ch;
    else if (depth > 1) topLevel += ' ';
  }
  const keys = new Set();
  for (const m of topLevel.matchAll(/(?:^|,)\s*([A-Za-z_$][\w$]*)\s*(?=[:,}]|$)/g)) keys.add(m[1]);
  return keys;
}

/** Findings for one frontend file. */
export function checkFrontendSource(source, file, commands) {
  const findings = [];
  const re = /invoke(?:<[^>]*>)?\(\s*['"](\w+)['"]\s*(,\s*\{)?/g;
  for (const match of source.matchAll(re)) {
    const name = match[1];
    const line = source.slice(0, match.index).split('\n').length;
    const at = `${file}:${line}`;
    const command = commands[name];
    if (!command) {
      findings.push(`${at} invokes '${name}', which the backend does not define`);
      continue;
    }
    if (!match[2]) continue;
    const keys = objectKeys(source, match.index + match[0].length);
    const accepted = new Set();
    for (const param of Object.keys(command.params)) {
      accepted.add(param);
      accepted.add(camel(param));
    }
    for (const key of keys) {
      if (!accepted.has(key)) {
        findings.push(`${at} passes '${key}' to '${name}', which takes: ${Object.keys(command.params).map(camel).join(', ') || 'nothing'}`);
      }
    }
    for (const [param, type] of Object.entries(command.params)) {
      if (type.startsWith('Option<')) continue;
      if (!keys.has(param) && !keys.has(camel(param))) {
        findings.push(`${at} calls '${name}' without its required '${camel(param)}'`);
      }
    }
  }
  return findings;
}

export function run(nativeRoot) {
  const commands = {};
  for (const file of walk(join(nativeRoot, 'src-tauri', 'src'), (n) => n.endsWith('.rs'))) {
    Object.assign(commands, parseRustCommands(readFileSync(file, 'utf8')));
  }
  const findings = [];
  const frontend = walk(join(nativeRoot, 'src'), (n) => /\.tsx?$/.test(n) && !/\.test\.tsx?$/.test(n));
  for (const file of frontend) {
    findings.push(...checkFrontendSource(readFileSync(file, 'utf8'), relative(nativeRoot, file), commands));
  }
  return { commandCount: Object.keys(commands).length, findings };
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const root = join(fileURLToPath(new URL('.', import.meta.url)), '..', 'native');
  const { commandCount, findings } = run(root);
  if (findings.length) {
    console.error(`✖ ${findings.length} frontend invoke call(s) do not match the ${commandCount} backend commands:`);
    for (const finding of findings) console.error(`  ${finding}`);
    process.exit(1);
  }
  console.log(`✅ Every frontend invoke matches one of the ${commandCount} backend commands.`);
}
