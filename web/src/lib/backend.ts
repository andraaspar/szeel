/** Calls into the Rust side of the app (web/src-tauri/src/main.rs). */
import { invoke } from '@tauri-apps/api/core'
import type { Analysis } from './types'

/** Parses a scene and outlines its paths. */
export function analyze(src: string): Promise<Analysis> {
  return invoke('analyze', { src })
}

export interface RawRender {
  width: number
  height: number
  ms: number
  pixels: Uint8ClampedArray<ArrayBuffer>
}

/** Renders a scene at `scale` times its size; null if it has errors. */
export async function render(src: string, scale: number): Promise<RawRender | null> {
  const buf = await invoke<ArrayBuffer>('render', { src, scale })
  if (buf.byteLength < 12) return null
  const head = new DataView(buf, 0, 12)
  const width = head.getUint32(0, true)
  const height = head.getUint32(4, true)
  const ms = head.getFloat32(8, true)
  return { width, height, ms, pixels: new Uint8ClampedArray(buf, 12, width * height * 4) }
}

/** Renders a scene at `scale` times its size and writes it to `path` as PNG. */
export function exportPng(src: string, scale: number, path: string): Promise<void> {
  return invoke('export_png', { src, scale, path })
}

export function readText(path: string): Promise<string> {
  return invoke('read_text', { path })
}

/** Writes `text` to `path`, replacing it whole. */
export function writeText(path: string, text: string): Promise<void> {
  return invoke('write_text', { path, text })
}

/** The scene file named on the command line, if any. */
export function initialFile(): Promise<string | null> {
  return invoke('initial_file')
}

/** Watches `path` for changes by other programs (the `file-changed` event); null stops watching. */
export function watch(path: string | null): Promise<void> {
  return invoke('watch', { path })
}

export function baseName(path: string): string {
  return path.split(/[\\/]/).pop() ?? path
}

export function dirName(path: string): string {
  const i = Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'))
  return i > 0 ? path.slice(0, i) : path
}
