<script lang="ts">
  import { onMount } from 'svelte'
  import Editor from './lib/Editor.svelte'
  import Preview from './lib/Preview.svelte'
  import { analyze } from './lib/wasm'
  import { Renderer } from './lib/renderer'
  import type { Rendered } from './lib/renderer'
  import { deletePoint, insertPoint, movePoint, newPath, pathAt } from './lib/edits'
  import type { Analysis, PointInfo } from './lib/types'

  // --- Examples and the initial document

  const examples = Object.fromEntries(
    Object.entries(
      import.meta.glob<string>('../../examples/*.kdl', { query: '?raw', import: 'default', eager: true }),
    ).map(([path, src]) => [path.split('/').pop()!, src]),
  )

  const BLANK = `image width=800 height=600

// Background: a closed loop, light inside, dark outside.
path closed=#true {
    point 100 100
    point 700 100
    point 700 500
    point 100 500
    gradient 1 { stop 0 0.3 0.05 260; stop 400 0.15 0.03 280; }
    gradient 3 { stop 0 0.85 0.04 80; stop 300 0.95 0.01 80; }
}
`

  const DRAFT_KEY = 'szeel-editor:draft'
  const SPLIT_KEY = 'szeel-editor:split'

  function loadDraft(): { text: string; name: string } {
    try {
      const d = JSON.parse(localStorage.getItem(DRAFT_KEY) ?? 'null')
      if (d && typeof d.text === 'string') return { text: d.text, name: String(d.name ?? 'untitled.kdl') }
    } catch {
      /* storage unavailable */
    }
    return { text: examples['rainbow.kdl'] ?? BLANK, name: 'rainbow.kdl' }
  }

  const initial = loadDraft()

  // --- State

  let text = $state(initial.text)
  let fileName = $state(initial.name)
  let fileHandle: FileSystemFileHandle | null = null
  let savedText = $state<string | null>(null)
  let analysis = $state<Analysis>(analyze(initial.text))
  let rendered = $state<Rendered | null>(null)
  let activePath = $state(-1)
  let activePoint = $state(-1)
  let showOverlay = $state(true)
  let showHidden = $state(true)
  let showHelp = $state(false)
  let exportScale = $state(1)
  let toast = $state<string | null>(null)
  let split = $state(readSplit())

  let editor: Editor
  let preview: Preview
  const renderer = new Renderer()

  const errors = $derived(analysis.diagnostics.filter((d) => d.severity === 'error').length)
  const warnings = $derived(analysis.diagnostics.length - errors)
  const dirty = $derived(savedText !== null && savedText !== text)

  // --- Rendering

  let dragging: { path: number; point: PointInfo } | null = null
  let lastFullMs = 0
  let fullTimer: ReturnType<typeof setTimeout> | undefined

  function show(r: Rendered | null) {
    if (!r) return
    rendered?.image.close()
    rendered = r
    if (r.scale === 1) lastFullMs = r.ms
  }

  /** A scale that renders in roughly interactive time. */
  function previewScale() {
    const px = analysis.width * analysis.height
    return px > 0 ? Math.min(1, Math.max(0.1, Math.sqrt(250_000 / px))) : 1
  }

  function requestRender() {
    if (!analysis.ok) return
    clearTimeout(fullTimer)
    const scale = previewScale()
    if ((dragging || lastFullMs > 90) && scale < 1) {
      renderer.render(text, scale).then(show)
      if (!dragging) fullTimer = setTimeout(() => renderer.render(text, 1).then(show), 250)
    } else {
      renderer.render(text, 1).then(show)
    }
  }

  let draftTimer: ReturnType<typeof setTimeout> | undefined
  function onTextChange(t: string) {
    text = t
    analysis = analyze(t)
    if (activePath >= analysis.paths.length) activePath = activePoint = -1
    requestRender()
    clearTimeout(draftTimer)
    draftTimer = setTimeout(() => {
      try {
        localStorage.setItem(DRAFT_KEY, JSON.stringify({ text, name: fileName }))
      } catch {
        /* storage unavailable */
      }
    }, 400)
  }

  function onCursor(pos: number) {
    if (dragging) return
    activePath = pathAt(analysis.paths, pos)
    activePoint =
      activePath < 0
        ? -1
        : analysis.paths[activePath].points.findIndex((p) => pos >= p.node[0] && pos <= p.node[1])
  }

  // --- Preview interactions

  function onSelect(pi: number, j: number) {
    if (pi < 0) {
      activePath = activePoint = -1
      return
    }
    const path = analysis.paths[pi]
    if (j >= 0) editor.select(path.points[j].node[0], path.points[j].node[0] + 5)
    else editor.select(path.name[0], path.name[1])
    activePath = pi
    activePoint = j
  }

  function onDragStart(pi: number, j: number) {
    dragging = { path: pi, point: analysis.paths[pi].points[j] }
    editor.beginTransient()
  }

  function onDrag(x: number, y: number, decimals: number) {
    if (!dragging) return
    editor.transient(movePoint(dragging.point, x, y, decimals))
  }

  function onDragEnd() {
    editor.commitTransient()
    dragging = null
    clearTimeout(fullTimer)
    if (analysis.ok) renderer.render(text, 1).then(show)
  }

  function decimalsOf(p: PointInfo) {
    return Number.isInteger(p.x) && Number.isInteger(p.y) ? 0 : 1
  }

  function onNudge(dx: number, dy: number) {
    const p = analysis.paths[activePath]?.points[activePoint]
    if (!p) return
    editor.apply(movePoint(p, p.x + dx, p.y + dy, decimalsOf(p)), p.node[0], p.node[0] + 5)
  }

  function onInsert(pi: number, after: number, x: number, y: number) {
    const change = insertPoint(text, analysis.paths[pi], after, Math.round(x), Math.round(y))
    editor.apply([change])
    const at = change.from + change.insert.indexOf('point')
    editor.select(at, at + 5)
  }

  function onDelete(pi: number, j: number) {
    const path = analysis.paths[pi]
    if (path.points.length <= 2) {
      flash('A path needs at least 2 points')
      return
    }
    editor.apply([deletePoint(text, path.points[j])])
    editor.select(path.name[0], path.name[1])
  }

  function addPath() {
    const c = preview.center()
    const size = Math.min(analysis.width, analysis.height) || 400
    const change = newPath(text, Math.round(c.x), Math.round(c.y), size)
    editor.apply([change])
    const at = change.from + change.insert.indexOf('path')
    editor.select(at, at + 4)
  }

  // --- Files

  function flash(msg: string) {
    toast = msg
    setTimeout(() => toast === msg && (toast = null), 2500)
  }

  function loadText(t: string, name: string, handle: FileSystemFileHandle | null = null) {
    fileName = name
    fileHandle = handle
    savedText = handle ? t : null
    lastFullMs = 0
    editor.load(t)
    activePath = activePoint = -1
    queueMicrotask(() => preview.fit())
  }

  function newScene() {
    loadText(BLANK, 'untitled.kdl')
  }

  function openExample(e: Event) {
    const sel = e.currentTarget as HTMLSelectElement
    if (sel.value) loadText(examples[sel.value], sel.value)
    sel.value = ''
  }

  let fileInput: HTMLInputElement

  async function openFile() {
    const w = window as unknown as { showOpenFilePicker?: (o: object) => Promise<FileSystemFileHandle[]> }
    if (w.showOpenFilePicker) {
      try {
        const [handle] = await w.showOpenFilePicker({
          types: [{ description: 'szeel scene', accept: { 'text/plain': ['.kdl'] } }],
        })
        const file = await handle.getFile()
        loadText(await file.text(), file.name, handle)
      } catch {
        /* cancelled */
      }
    } else fileInput.click()
  }

  async function onFilePicked() {
    const file = fileInput.files?.[0]
    if (file) loadText(await file.text(), file.name)
    fileInput.value = ''
  }

  function download(blob: Blob, name: string) {
    const a = document.createElement('a')
    a.href = URL.createObjectURL(blob)
    a.download = name
    a.click()
    setTimeout(() => URL.revokeObjectURL(a.href), 1000)
  }

  async function save() {
    const w = window as unknown as { showSaveFilePicker?: (o: object) => Promise<FileSystemFileHandle> }
    const snapshot = text
    try {
      if (!fileHandle && w.showSaveFilePicker) {
        fileHandle = await w.showSaveFilePicker({
          suggestedName: fileName,
          types: [{ description: 'szeel scene', accept: { 'text/plain': ['.kdl'] } }],
        })
        fileName = fileHandle.name
      }
      if (fileHandle) {
        const out = await fileHandle.createWritable()
        await out.write(snapshot)
        await out.close()
        savedText = snapshot
        flash(`Saved ${fileName}`)
      } else {
        download(new Blob([snapshot], { type: 'text/plain' }), fileName)
      }
    } catch {
      /* cancelled */
    }
  }

  async function exportPng() {
    if (!analysis.ok) return flash('Fix the errors first')
    const r = await renderer.render(text, exportScale)
    if (!r) return flash('Render failed')
    const c = document.createElement('canvas')
    c.width = r.image.width
    c.height = r.image.height
    c.getContext('2d')!.drawImage(r.image, 0, 0)
    r.image.close()
    c.toBlob((blob) => blob && download(blob, fileName.replace(/\.kdl$/i, '') + (exportScale !== 1 ? `@${exportScale}x` : '') + '.png'))
  }

  // --- Layout

  function readSplit() {
    try {
      const v = Number(localStorage.getItem(SPLIT_KEY))
      if (v > 0.1 && v < 0.9) return v
    } catch {
      /* storage unavailable */
    }
    return 0.4
  }

  let main: HTMLElement
  function onSplitDown(e: PointerEvent) {
    const el = e.currentTarget as HTMLElement
    el.setPointerCapture(e.pointerId)
    const move = (ev: PointerEvent) => {
      const r = main.getBoundingClientRect()
      split = Math.min(0.8, Math.max(0.15, (ev.clientX - r.left) / r.width))
    }
    const up = () => {
      el.removeEventListener('pointermove', move)
      el.removeEventListener('pointerup', up)
      try {
        localStorage.setItem(SPLIT_KEY, String(split))
      } catch {
        /* storage unavailable */
      }
    }
    el.addEventListener('pointermove', move)
    el.addEventListener('pointerup', up)
  }

  function onGlobalKey(e: KeyboardEvent) {
    const mod = e.ctrlKey || e.metaKey
    if (mod && e.key === 's') {
      e.preventDefault()
      save()
    } else if (mod && e.key === 'o') {
      e.preventDefault()
      openFile()
    } else if (mod && e.shiftKey && e.key.toLowerCase() === 'e') {
      e.preventDefault()
      exportPng()
    }
  }

  onMount(() => requestRender())
</script>

<svelte:window onkeydown={onGlobalKey} />

<header>
  <div class="brand">szeel</div>
  <div class="file" title={fileName}>{fileName}{dirty ? ' •' : ''}</div>

  <div class="group">
    <button onclick={newScene}>New</button>
    <button onclick={openFile} title="Ctrl+O">Open…</button>
    <button onclick={save} title="Ctrl+S">Save</button>
    <select onchange={openExample} aria-label="Open an example">
      <option value="">Examples…</option>
      {#each Object.keys(examples) as name}<option value={name}>{name}</option>{/each}
    </select>
  </div>

  <div class="group">
    <button onclick={addPath} title="Add a path in front of everything, at the centre of the view">+ Path</button>
  </div>

  <div class="group">
    <label class="check"><input type="checkbox" bind:checked={showOverlay} /> Overlay</label>
    <label class="check" class:disabled={!showOverlay}>
      <input type="checkbox" bind:checked={showHidden} disabled={!showOverlay} /> Masks &amp; defs
    </label>
  </div>

  <div class="group">
    <button onclick={() => preview.zoomBy(1 / 1.25)} aria-label="Zoom out">−</button>
    <button onclick={() => preview.fit()}>Fit</button>
    <button onclick={() => preview.actualSize()}>100%</button>
    <button onclick={() => preview.zoomBy(1.25)} aria-label="Zoom in">+</button>
  </div>

  <div class="spacer"></div>

  <div class="group">
    <select bind:value={exportScale} aria-label="Export scale">
      <option value={0.5}>0.5×</option>
      <option value={1}>1×</option>
      <option value={2}>2×</option>
      <option value={4}>4×</option>
    </select>
    <button class="primary" onclick={exportPng} disabled={!analysis.ok} title="Ctrl+Shift+E">Export PNG</button>
  </div>
  <button class="icon" onclick={() => (showHelp = !showHelp)} aria-label="Help" aria-expanded={showHelp}>?</button>
</header>

<main bind:this={main} style:grid-template-columns="{split * 100}% 6px 1fr">
  <section class="left">
    <div class="code">
      <Editor
        bind:this={editor}
        doc={initial.text}
        diagnostics={analysis.diagnostics}
        onchange={onTextChange}
        oncursor={onCursor}
      />
    </div>
    <div class="problems" class:has-errors={errors > 0}>
      <div class="summary">
        {#if analysis.diagnostics.length === 0}
          <span class="ok">✓ No problems</span>
          <span class="dim">{analysis.width}×{analysis.height} · {analysis.paths.length} paths</span>
        {:else}
          <span>{errors} error{errors === 1 ? '' : 's'}, {warnings} warning{warnings === 1 ? '' : 's'}</span>
        {/if}
      </div>
      {#if analysis.diagnostics.length}
        <ul>
          {#each analysis.diagnostics as d}
            <li>
              <button class="diag {d.severity}" onclick={() => { editor.select(d.range[0], d.range[1]); editor.focus() }}>
                <span class="sev">{d.severity}</span>
                <span class="msg">{d.message}{d.label ? ` — ${d.label}` : ''}</span>
                {#if d.help}<span class="help">{d.help}</span>{/if}
              </button>
            </li>
          {/each}
        </ul>
      {/if}
    </div>
  </section>

  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="splitter" onpointerdown={onSplitDown}></div>

  <section class="right">
    <Preview
      bind:this={preview}
      {analysis}
      {rendered}
      stale={!analysis.ok}
      {activePath}
      {activePoint}
      {showOverlay}
      {showHidden}
      onselect={onSelect}
      ondragstart={onDragStart}
      ondrag={onDrag}
      ondragend={onDragEnd}
      oninsert={onInsert}
      ondelete={onDelete}
      onnudge={onNudge}
    />
    {#if showHelp}
      <aside class="help-panel">
        <h3>Preview</h3>
        <dl>
          <dt>Drag a point</dt><dd>Move it (Shift: 0.1 px steps)</dd>
          <dt>Click a curve</dt><dd>Select its path</dd>
          <dt>Double-click a curve</dt><dd>Insert a point there</dd>
          <dt>Alt+click a point</dt><dd>Delete it</dd>
          <dt>Arrows</dt><dd>Nudge the selected point (Shift: 10 px)</dd>
          <dt>Delete</dt><dd>Delete the selected point</dd>
          <dt>Wheel / pinch</dt><dd>Zoom at the cursor</dd>
          <dt>Drag background</dt><dd>Pan</dd>
        </dl>
        <h3>Overlay</h3>
        <dl>
          <dt><span class="swatch draw"></span>Solid</dt><dd>Drawn paths</dd>
          <dt><span class="swatch mask"></span>Dashed</dt><dd>Mask paths</dd>
          <dt><span class="swatch defs"></span>Dotted</dt><dd>Paths in <code>defs</code></dd>
          <dt>Thick ring</dt><dd>First point of a path</dd>
        </dl>
        <h3>Keys</h3>
        <dl>
          <dt>Ctrl+S / Ctrl+O</dt><dd>Save / open</dd>
          <dt>Ctrl+Shift+E</dt><dd>Export PNG</dd>
          <dt>Ctrl+Z</dt><dd>Undo (a whole drag is one step)</dd>
        </dl>
      </aside>
    {/if}
    {#if toast}<div class="toast">{toast}</div>{/if}
  </section>
</main>

<input type="file" accept=".kdl,text/plain" bind:this={fileInput} onchange={onFilePicked} hidden />

<style>
  header {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 10px;
    background: var(--panel-2);
    border-bottom: 1px solid var(--border);
    flex-wrap: wrap;
  }
  .brand {
    font-weight: 700;
    letter-spacing: 0.04em;
    color: var(--accent);
  }
  .file {
    color: var(--muted);
    font: 12px var(--mono);
    max-width: 18ch;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .group {
    display: flex;
    align-items: center;
    gap: 4px;
    padding-left: 10px;
    border-left: 1px solid var(--border);
  }
  .spacer {
    flex: 1;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: 12px;
    color: var(--text);
    padding: 0 4px;
    cursor: pointer;
  }
  .check.disabled {
    opacity: 0.5;
  }
  main {
    flex: 1;
    min-height: 0;
    display: grid;
  }
  .left {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
  }
  .code {
    flex: 1;
    min-height: 0;
  }
  .splitter {
    background: var(--border);
    cursor: col-resize;
  }
  .splitter:hover {
    background: var(--accent);
  }
  .right {
    position: relative;
    min-width: 0;
    min-height: 0;
  }
  .problems {
    border-top: 1px solid var(--border);
    background: var(--panel-2);
    max-height: 30%;
    overflow: auto;
    font-size: 12px;
  }
  .summary {
    display: flex;
    justify-content: space-between;
    padding: 6px 10px;
    color: var(--muted);
  }
  .has-errors .summary {
    color: var(--error);
  }
  .ok {
    color: var(--ok);
  }
  .dim {
    color: var(--muted);
  }
  .problems ul {
    list-style: none;
    margin: 0;
    padding: 0 0 4px;
  }
  .diag {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 2px 8px;
    width: 100%;
    text-align: left;
    background: none;
    border: none;
    border-radius: 0;
    padding: 4px 10px;
    color: var(--text);
  }
  .diag:hover {
    background: var(--active-line);
  }
  .sev {
    font: 600 10px var(--mono);
    text-transform: uppercase;
    padding-top: 2px;
  }
  .diag.error .sev {
    color: var(--error);
  }
  .diag.warning .sev {
    color: var(--warning);
  }
  .help {
    grid-column: 2;
    color: var(--muted);
  }
  .help-panel {
    position: absolute;
    top: 10px;
    right: 10px;
    width: 300px;
    max-height: calc(100% - 60px);
    overflow: auto;
    background: var(--panel-2);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 4px 14px 10px;
    font-size: 12px;
    box-shadow: 0 8px 30px rgb(0 0 0 / 0.4);
  }
  .help-panel h3 {
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--muted);
    margin: 12px 0 6px;
  }
  .help-panel dl {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 4px 12px;
    margin: 0;
  }
  .help-panel dt {
    color: var(--text);
    font-weight: 600;
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .help-panel dd {
    margin: 0;
    color: var(--muted);
  }
  .swatch {
    width: 16px;
    height: 0;
    border-top: 2px solid;
  }
  .swatch.draw {
    border-color: var(--role-draw);
  }
  .swatch.mask {
    border-color: var(--role-mask);
    border-top-style: dashed;
  }
  .swatch.defs {
    border-color: var(--role-defs);
    border-top-style: dotted;
  }
  .toast {
    position: absolute;
    left: 50%;
    bottom: 16px;
    transform: translateX(-50%);
    background: var(--panel-2);
    border: 1px solid var(--border);
    padding: 6px 12px;
    border-radius: 6px;
    font-size: 12px;
  }
</style>
