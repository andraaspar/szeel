<script lang="ts">
  import { onMount } from 'svelte'
  import type { Analysis, PathInfo } from './types'
  import type { Rendered } from './renderer'
  import { segmentStarts } from './edits'

  interface Props {
    analysis: Analysis
    rendered: Rendered | null
    stale: boolean
    activePath: number
    activePoint: number
    showOverlay: boolean
    showHidden: boolean
    onselect: (path: number, point: number) => void
    ondragstart: (path: number, point: number) => void
    ondrag: (x: number, y: number, decimals: number) => void
    ondragend: () => void
    oninsert: (path: number, after: number, x: number, y: number) => void
    ondelete: (path: number, point: number) => void
    oncusp: (path: number, point: number) => void
    onnudge: (dx: number, dy: number) => void
  }
  let {
    analysis,
    rendered,
    stale,
    activePath,
    activePoint,
    showOverlay,
    showHidden,
    onselect,
    ondragstart,
    ondrag,
    ondragend,
    oninsert,
    ondelete,
    oncusp,
    onnudge,
  }: Props = $props()

  let viewport: HTMLDivElement
  let canvas: HTMLCanvasElement
  let vw = $state(0)
  let vh = $state(0)
  let zoom = $state(1)
  let tx = $state(0)
  let ty = $state(0)
  let fitted = false

  const w = $derived(analysis.width)
  const h = $derived(analysis.height)

  // Draw each finished render into the canvas, stretched to the scene size.
  $effect(() => {
    if (!rendered || !canvas) return
    if (canvas.width !== w || canvas.height !== h) {
      canvas.width = w
      canvas.height = h
    }
    const ctx = canvas.getContext('2d')!
    ctx.clearRect(0, 0, w, h)
    ctx.imageSmoothingEnabled = true
    ctx.imageSmoothingQuality = 'high'
    ctx.drawImage(rendered.image, 0, 0, w, h)
  })

  // Fit the first time a scene with a size appears, and whenever the size changes.
  let lastSize = ''
  $effect(() => {
    const size = `${w}x${h}`
    if (w > 0 && h > 0 && vw > 0 && (size !== lastSize || !fitted)) {
      lastSize = size
      fitted = true
      fit()
    }
  })

  export function fit() {
    if (!w || !h || !vw || !vh) return
    const pad = 32
    zoom = Math.max(0.02, Math.min((vw - pad * 2) / w, (vh - pad * 2) / h))
    tx = (vw - w * zoom) / 2
    ty = (vh - h * zoom) / 2
  }

  export function actualSize() {
    zoomAt(vw / 2, vh / 2, 1 / zoom)
  }

  function zoomAt(sx: number, sy: number, factor: number) {
    const z = Math.min(64, Math.max(0.02, zoom * factor))
    const f = z / zoom
    tx = sx - (sx - tx) * f
    ty = sy - (sy - ty) * f
    zoom = z
  }

  export function zoomBy(factor: number) {
    zoomAt(vw / 2, vh / 2, factor)
  }

  export function getZoom() {
    return zoom
  }

  export function center() {
    return toScene(vw / 2, vh / 2)
  }

  function local(e: { clientX: number; clientY: number }) {
    const r = viewport.getBoundingClientRect()
    return { x: e.clientX - r.left, y: e.clientY - r.top }
  }

  function toScene(sx: number, sy: number) {
    return { x: (sx - tx) / zoom, y: (sy - ty) / zoom }
  }

  const sx = (x: number) => tx + x * zoom
  const sy = (y: number) => ty + y * zoom

  function d(path: PathInfo) {
    return path.curves
      .map((c, i) => {
        const head = i === 0 ? `M${sx(c[0])} ${sy(c[1])}` : ''
        return `${head}C${sx(c[2])} ${sy(c[3])} ${sx(c[4])} ${sy(c[5])} ${sx(c[6])} ${sy(c[7])}`
      })
      .join('')
  }

  function segD(c: number[]) {
    return `M${sx(c[0])} ${sy(c[1])}C${sx(c[2])} ${sy(c[3])} ${sx(c[4])} ${sy(c[5])} ${sx(c[6])} ${sy(c[7])}`
  }

  function circle(x: number, y: number, r: number) {
    return `M${x - r} ${y}a${r} ${r} 0 1 0 ${2 * r} 0a${r} ${r} 0 1 0 ${-2 * r} 0Z`
  }

  function diamond(x: number, y: number, r: number) {
    return `M${x} ${y - r}L${x + r} ${y}L${x} ${y + r}L${x - r} ${y}Z`
  }

  const visible = $derived(
    analysis.paths
      .map((p, i) => ({ p, i }))
      .filter(({ p, i }) => showHidden || p.role === 'draw' || i === activePath),
  )

  // --- Pointer handling

  type Gesture =
    | { kind: 'pan'; x: number; y: number; tx: number; ty: number; moved: boolean }
    | { kind: 'point'; dx: number; dy: number; moved: boolean }
  let gesture: Gesture | null = null

  function onWheel(e: WheelEvent) {
    e.preventDefault()
    const p = local(e)
    if (e.ctrlKey || e.metaKey || !e.shiftKey) {
      // Pinch gestures arrive as ctrl+wheel with small deltas.
      const k = e.ctrlKey ? 0.01 : 0.0015
      zoomAt(p.x, p.y, Math.exp(-e.deltaY * k))
    } else {
      tx -= e.deltaX || e.deltaY
    }
  }

  function onBackgroundDown(e: PointerEvent) {
    if (e.button !== 0 && e.button !== 1) return
    viewport.focus()
    const p = local(e)
    gesture = { kind: 'pan', x: p.x, y: p.y, tx, ty, moved: false }
    viewport.setPointerCapture(e.pointerId)
  }

  function onHandleDown(e: PointerEvent, pi: number, i: number) {
    if (e.button !== 0) return
    e.stopPropagation()
    viewport.focus()
    if (e.altKey) {
      ondelete(pi, i)
      return
    }
    const pt = analysis.paths[pi].points[i]
    const s = toScene(local(e).x, local(e).y)
    gesture = { kind: 'point', dx: pt.x - s.x, dy: pt.y - s.y, moved: false }
    onselect(pi, i)
    ondragstart(pi, i)
    viewport.setPointerCapture(e.pointerId)
  }

  function onCurveDown(e: PointerEvent, pi: number) {
    if (e.button !== 0) return
    e.stopPropagation()
    viewport.focus()
    onselect(pi, -1)
  }

  function onCurveDblClick(e: MouseEvent, pi: number, seg: number) {
    e.stopPropagation()
    const s = toScene(local(e).x, local(e).y)
    const after = segmentStarts(analysis.paths[pi])[seg] ?? seg
    oninsert(pi, after, s.x, s.y)
  }

  function onMove(e: PointerEvent) {
    if (!gesture) return
    const p = local(e)
    if (gesture.kind === 'pan') {
      if (Math.abs(p.x - gesture.x) + Math.abs(p.y - gesture.y) > 3) gesture.moved = true
      tx = gesture.tx + p.x - gesture.x
      ty = gesture.ty + p.y - gesture.y
    } else {
      gesture.moved = true
      const decimals = e.shiftKey || zoom >= 4 ? 1 : 0
      const s = toScene(p.x, p.y)
      ondrag(s.x + gesture.dx, s.y + gesture.dy, decimals)
    }
  }

  function onUp(e: PointerEvent) {
    if (!gesture) return
    if (gesture.kind === 'pan' && !gesture.moved && e.button === 0) onselect(-1, -1)
    if (gesture.kind === 'point') ondragend()
    gesture = null
    if (viewport.hasPointerCapture(e.pointerId)) viewport.releasePointerCapture(e.pointerId)
  }

  function onKey(e: KeyboardEvent) {
    if (activePath >= 0 && activePoint >= 0) {
      const step = e.shiftKey ? 10 : 1
      const dirs: Record<string, [number, number]> = {
        ArrowLeft: [-step, 0],
        ArrowRight: [step, 0],
        ArrowUp: [0, -step],
        ArrowDown: [0, step],
      }
      if (dirs[e.key]) {
        e.preventDefault()
        onnudge(...dirs[e.key])
        return
      }
      if (e.key === 'Delete' || e.key === 'Backspace') {
        e.preventDefault()
        ondelete(activePath, activePoint)
        return
      }
      if ((e.key === 'c' || e.key === 'C') && !e.ctrlKey && !e.metaKey && !e.altKey) {
        e.preventDefault()
        oncusp(activePath, activePoint)
        return
      }
    }
    if (e.key === 'Escape') onselect(-1, -1)
  }

  onMount(() => {
    const ro = new ResizeObserver(([entry]) => {
      vw = entry.contentRect.width
      vh = entry.contentRect.height
    })
    ro.observe(viewport)
    viewport.addEventListener('wheel', onWheel, { passive: false })
    return () => {
      ro.disconnect()
      viewport.removeEventListener('wheel', onWheel)
    }
  })
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<div
  class="viewport"
  bind:this={viewport}
  tabindex="0"
  role="application"
  aria-label="Scene preview"
  onpointerdown={onBackgroundDown}
  onpointermove={onMove}
  onpointerup={onUp}
  onpointercancel={onUp}
  onkeydown={onKey}
>
  <div
    class="stage"
    class:stale
    class:pixelated={zoom >= 3}
    style:width="{w}px"
    style:height="{h}px"
    style:transform="translate({tx}px, {ty}px) scale({zoom})"
    style:--checker="{Math.max(4, 12 / zoom)}px"
  >
    <canvas bind:this={canvas}></canvas>
  </div>

  {#if showOverlay}
    <svg class="overlay" width={vw} height={vh}>
      <rect class="frame" x={sx(0)} y={sy(0)} width={w * zoom} height={h * zoom} />
      {#each visible as { p, i } (i)}
        <g class="path role-{p.role}" class:active={i === activePath}>
          <path class="curve-shadow" d={d(p)} />
          <path class="curve" d={d(p)} />
          {#each p.curves as c, s}
            <!-- svelte-ignore a11y_no_static_element_interactions -->
            <path
              class="hit"
              d={segD(c)}
              onpointerdown={(e) => onCurveDown(e, i)}
              ondblclick={(e) => onCurveDblClick(e, i, s)}
            />
          {/each}
        </g>
      {/each}
      {#each visible as { p, i } (i)}
        {#if i === activePath || activePath < 0}
          {#each p.points as pt, j}
            {@const r = i === activePath ? 6 : 4}
            <!-- Cusps are diamonds; smooth points are circles. -->
            <!-- svelte-ignore a11y_no_static_element_interactions -->
            <path
              class="handle role-{p.role}"
              class:active-path={i === activePath}
              class:selected={i === activePath && j === activePoint}
              class:first={j === 0}
              d={pt.cusp ? diamond(sx(pt.x), sy(pt.y), r * 1.3) : circle(sx(pt.x), sy(pt.y), r)}
              onpointerdown={(e) => onHandleDown(e, i, j)}
              ondblclick={(e) => {
                e.stopPropagation()
                oncusp(i, j)
              }}
            >
              <title
                >{p.id ? `${p.id} · ` : ''}point {j + 1}: {pt.x}, {pt.y}{pt.cusp ? ' · cusp' : ''} (double-click or C to toggle cusp)</title
              >
            </path>
          {/each}
        {/if}
      {/each}
    </svg>
  {/if}

  <div class="hud">
    <span>{Math.round(zoom * 100)}%</span>
    {#if rendered}<span title="Render time">{rendered.ms.toFixed(0)} ms{rendered.scale < 1 ? ` @ ${Math.round(rendered.scale * 100)}%` : ''}</span>{/if}
  </div>
</div>

<style>
  .viewport {
    position: relative;
    width: 100%;
    height: 100%;
    overflow: hidden;
    background: var(--canvas-bg);
    cursor: grab;
    outline: none;
    touch-action: none;
    user-select: none;
  }
  .viewport:active {
    cursor: grabbing;
  }
  .viewport:focus-visible {
    box-shadow: inset 0 0 0 2px var(--accent);
  }
  .stage {
    position: absolute;
    left: 0;
    top: 0;
    transform-origin: 0 0;
    background-color: #fff;
    background-image:
      linear-gradient(45deg, #d8d8d8 25%, transparent 25%),
      linear-gradient(-45deg, #d8d8d8 25%, transparent 25%),
      linear-gradient(45deg, transparent 75%, #d8d8d8 75%),
      linear-gradient(-45deg, transparent 75%, #d8d8d8 75%);
    background-size: calc(var(--checker) * 2) calc(var(--checker) * 2);
    background-position:
      0 0,
      0 var(--checker),
      var(--checker) calc(var(--checker) * -1),
      calc(var(--checker) * -1) 0;
    box-shadow: 0 4px 24px rgb(0 0 0 / 0.35);
    transition: filter 0.2s, opacity 0.2s;
  }
  .stage.stale {
    filter: grayscale(0.8);
    opacity: 0.45;
  }
  .stage.pixelated canvas {
    image-rendering: pixelated;
  }
  canvas {
    display: block;
    width: 100%;
    height: 100%;
  }
  .overlay {
    position: absolute;
    inset: 0;
    pointer-events: none;
  }
  .frame {
    fill: none;
    stroke: rgb(255 255 255 / 0.15);
    stroke-width: 1;
  }
  .curve,
  .curve-shadow {
    fill: none;
  }
  .curve-shadow {
    stroke: rgb(0 0 0 / 0.45);
    stroke-width: 3;
  }
  .curve {
    stroke: var(--c);
    stroke-width: 1.25;
    opacity: 0.75;
  }
  .path {
    --c: var(--role-draw);
  }
  .path.role-mask {
    --c: var(--role-mask);
  }
  .path.role-mask .curve {
    stroke-dasharray: 6 4;
  }
  .path.role-defs {
    --c: var(--role-defs);
  }
  .path.role-defs .curve {
    stroke-dasharray: 2 4;
  }
  .path.active .curve {
    stroke-width: 2;
    opacity: 1;
  }
  .path.active .curve-shadow {
    stroke-width: 4.5;
  }
  .hit {
    fill: none;
    stroke: transparent;
    stroke-width: 12;
    pointer-events: stroke;
    cursor: pointer;
  }
  .handle {
    --c: var(--role-draw);
    fill: var(--handle-fill);
    stroke: var(--c);
    stroke-width: 1.5;
    pointer-events: all;
    cursor: move;
  }
  .handle.role-mask {
    --c: var(--role-mask);
  }
  .handle.role-defs {
    --c: var(--role-defs);
  }
  .handle.first {
    stroke-width: 3;
  }
  .handle.active-path {
    stroke-width: 2;
  }
  .handle.selected {
    fill: var(--c);
    stroke: #fff;
  }
  .handle:hover {
    stroke: #fff;
  }
  .hud {
    position: absolute;
    right: 10px;
    bottom: 10px;
    display: flex;
    gap: 6px;
    font: 11px var(--mono);
    pointer-events: none;
  }
  .hud span {
    background: rgb(0 0 0 / 0.55);
    color: #ddd;
    padding: 3px 7px;
    border-radius: 4px;
  }
</style>
