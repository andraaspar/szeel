import { render } from './backend'

export interface Rendered {
  image: ImageBitmap
  /** Scale the image was rendered at, relative to the scene size. */
  scale: number
  ms: number
}

/**
 * Renders scenes on the Rust side. Only one render runs at a time; a request
 * made while one is running replaces any other waiting request, so the
 * preview always catches up with the latest text without queueing.
 */
export class Renderer {
  private running = false
  private waiting: { src: string; scale: number; resolve: (r: Rendered | null) => void } | null = null

  /** Renders `src`; resolves to null if superseded by a later request or if the scene has errors. */
  render(src: string, scale = 1): Promise<Rendered | null> {
    return new Promise((resolve) => {
      this.waiting?.resolve(null)
      this.waiting = { src, scale, resolve }
      this.pump()
    })
  }

  private async pump() {
    if (this.running || !this.waiting) return
    const { src, scale, resolve } = this.waiting
    this.waiting = null
    this.running = true
    let raw
    try {
      raw = await render(src, scale)
    } catch (e) {
      console.error('render failed', e)
      raw = null
    }
    this.running = false
    this.pump()
    if (!raw) return resolve(null)
    const image = await createImageBitmap(new ImageData(raw.pixels, raw.width, raw.height))
    resolve({ image, scale, ms: raw.ms })
  }
}
