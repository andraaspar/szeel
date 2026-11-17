/// <reference lib="webworker" />
import init, { render } from '../wasm/szeel_wasm.js'

export interface RenderRequest {
  id: number
  src: string
  scale: number
}

export type RenderResponse =
  | { id: number; ok: true; width: number; height: number; pixels: Uint8ClampedArray<ArrayBuffer>; ms: number }
  | { id: number; ok: false }

const ready = init()

self.onmessage = async (e: MessageEvent<RenderRequest>) => {
  await ready
  const { id, src, scale } = e.data
  const t0 = performance.now()
  const out = render(src, scale)
  const ms = performance.now() - t0
  if (out.length < 8) {
    self.postMessage({ id, ok: false } satisfies RenderResponse)
    return
  }
  const head = new DataView(out.buffer, out.byteOffset, 8)
  const width = head.getUint32(0, true)
  const height = head.getUint32(4, true)
  const pixels = new Uint8ClampedArray(out.buffer, out.byteOffset + 8, width * height * 4).slice()
  self.postMessage({ id, ok: true, width, height, pixels, ms } satisfies RenderResponse, [pixels.buffer])
}
