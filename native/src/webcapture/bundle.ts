/**
 * The content script's file name, shared by the build that emits it
 * (`vite.extension.config.ts`) and the background worker that injects it.
 *
 * Two literals used to name it separately, and after a rename they
 * disagreed: every capture failed with "Could not load file".
 */
export const CONTENT_BUNDLE_NAME = 'vox-extract';

/** The emitted file, as `chrome.scripting.executeScript` names it. */
export const CONTENT_BUNDLE_FILE = `${CONTENT_BUNDLE_NAME}.js`;
