/** Display policy — the single owner of the injected-context visibility toggle.
 *
 * Debug mode is off by default and persists only for the lifetime of the Pi
 * process (no daemon round-trip, no persistence). Toggle with `/mosaico debug`.
 */
let debugEnabled = false

/** Flip the debug toggle and return the new state. */
export function toggleDebug(): boolean {
  debugEnabled = !debugEnabled
  return debugEnabled
}

/** Whether injected context messages should be rendered in the transcript. */
export function isDebugEnabled(): boolean {
  return debugEnabled
}
