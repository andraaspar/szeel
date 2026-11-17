import type { RenderRequest, RenderResponse } from './render-worker'

export interface Rendered {
  image: ImageBitmap
  /** Scale the image was rendered at, relative to the scene size. */
  scale: number
  ms: number
}

/**
 * Renders scenes in a Web Worker. Only one render runs at a time; a request
 * made while one is running replaces any other waiting request, so the
 * preview always catches up with the latest text without queueing.
 */
export class Renderer {
  private worker = new Worker(new URL('./render-worker.ts', import.meta.url), { type: 'module' })
  private nextId = 1
  private running: { id: number; scale: number; resolve: (r: Rendered | null) => void } | null = null
  private waiting: { src: string; scale: number; resolve: (r: Rendered | null) => void } | null = null

  constructor() {
    this.worker.onmessage = async (e: MessageEvent<RenderResponse>) => {
      const job = this.running
      this.running = null
      this.pump()
      if (!job || job.id !== e.data.id) return
      const d = e.data
      if (!d.ok) return job.resolve(null)
      const image = await createImageBitmap(new ImageData(d.pixels, d.width, d.height))
      job.resolve({ image, scale: job.scale, ms: d.ms })
    }
  }

  /** Renders `src`; resolves to null if superseded by a later request or if the scene has errors. */
  render(src: string, scale = 1): Promise<Rendered | null> {
    return new Promise((resolve) => {
      this.waiting?.resolve(null)
      this.waiting = { src, scale, resolve }
      this.pump()
    })
  }

  private pump() {
    if (this.running || !this.waiting) return
    const { src, scale, resolve } = this.waiting
    this.waiting = null
    const id = this.nextId++
    this.running = { id, scale, resolve }
    this.worker.postMessage({ id, src, scale } satisfies RenderRequest)
  }
}
