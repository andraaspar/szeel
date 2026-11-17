import init, { analyze as rawAnalyze } from '../wasm/szeel_wasm.js'
import type { Analysis } from './types'

/** Resolves once the WebAssembly module is ready on the main thread. */
export const ready: Promise<void> = init().then(() => undefined)

/** Parses a scene and outlines its paths. Call only after `ready`. */
export function analyze(src: string): Analysis {
  return JSON.parse(rawAnalyze(src)) as Analysis
}
