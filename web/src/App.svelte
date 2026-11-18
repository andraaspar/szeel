<script lang="ts">
  import { onMount } from 'svelte'
  import { listen } from '@tauri-apps/api/event'
  import { getCurrentWindow } from '@tauri-apps/api/window'
  import { getCurrentWebview } from '@tauri-apps/api/webview'
  import { message, open, save } from '@tauri-apps/plugin-dialog'
  import Editor from './lib/Editor.svelte'
  import Preview from './lib/Preview.svelte'
  import * as backend from './lib/backend'
  import { installMenu } from './lib/menu'
  import type { AppMenu } from './lib/menu'
  import { baseName, dirName } from './lib/backend'
  import { Renderer } from './lib/renderer'
  import type { Rendered } from './lib/renderer'
  import { deletePoint, insertPoint, layoutChanges, movePoint, newPath, pathAt } from './lib/edits'
  import type { Analysis, PointInfo } from './lib/types'

  // --- Examples and new scenes

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
    gradient 1 {
        stop 0 0.3 0.05 260
        stop 400 0.15 0.03 280
    }
    gradient 3 {
        stop 0 0.85 0.04 80
        stop 300 0.95 0.01 80
    }
}
`

  const EMPTY: Analysis = { ok: false, width: 0, height: 0, diagnostics: [], paths: [] }

  // --- Stored between sessions

  const DRAFT_KEY = 'szeel-editor:draft'
  const SPLIT_KEY = 'szeel-editor:split'
  const RECENT_KEY = 'szeel-editor:recent'
  const MAX_RECENT = 10

  /** The text being edited, kept so a crash or a forced close loses nothing. */
  interface Draft {
    text: string
    path: string | null
    name: string
  }

  function stored<T>(key: string): T | null {
    try {
      return JSON.parse(localStorage.getItem(key) ?? 'null') as T | null
    } catch {
      return null
    }
  }

  function store(key: string, value: unknown) {
    try {
      localStorage.setItem(key, JSON.stringify(value))
    } catch {
      /* storage unavailable */
    }
  }

  // --- State

  let text = $state('')
  /** The file being edited; null for a scene never saved. */
  let filePath = $state<string | null>(null)
  /** Name shown for a scene never saved. */
  let untitledName = $state('untitled.kdl')
  /** The text as last loaded or saved; null if it differs from everything on disk. */
  let savedText = $state<string | null>('')
  let analysis = $state<Analysis>(EMPTY)
  let rendered = $state<Rendered | null>(null)
  let activePath = $state(-1)
  let activePoint = $state(-1)
  let showOverlay = $state(true)
  let showHidden = $state(true)
  let exportScale = $state(1)
  let toast = $state<string | null>(null)
  let split = $state(readSplit())
  let recent = $state<string[]>(stored<string[]>(RECENT_KEY)?.filter((p) => typeof p === 'string') ?? [])

  let editor: Editor
  let preview: Preview
  const renderer = new Renderer()

  const fileName = $derived(filePath ? baseName(filePath) : untitledName)
  const errors = $derived(analysis.diagnostics.filter((d) => d.severity === 'error').length)
  const warnings = $derived(analysis.diagnostics.length - errors)
  const dirty = $derived(savedText !== text)

  $effect(() => {
    getCurrentWindow().setTitle(`${dirty ? '• ' : ''}${fileName}${filePath ? ` — ${dirName(filePath)}` : ''} — szeel`)
  })

  // --- Analysis and rendering

  /** The text `analysis` describes. Its ranges index into this, not into `text`, until they match. */
  let analyzed = ''
  let analyzeSeq = 0
  let cursor = 0

  /** Whether `analysis` describes the current text, so its ranges can be used to edit it. */
  const current = () => analyzed === text

  async function reanalyze(t: string) {
    const seq = ++analyzeSeq
    const a = await backend.analyze(t)
    if (seq !== analyzeSeq) return
    analysis = a
    analyzed = t
    if (!dragging) onCursor(cursor)
    requestRender()
  }

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
    return px > 0 ? Math.min(1, Math.max(0.1, Math.sqrt(1_000_000 / px))) : 1
  }

  function requestRender() {
    if (!analysis.ok) return
    const src = analyzed
    clearTimeout(fullTimer)
    const scale = previewScale()
    if ((dragging || lastFullMs > 60) && scale < 1) {
      renderer.render(src, scale).then(show)
      if (!dragging) fullTimer = setTimeout(() => renderer.render(src, 1).then(show), 250)
    } else {
      renderer.render(src, 1).then(show)
    }
  }

  let draftTimer: ReturnType<typeof setTimeout> | undefined
  function onTextChange(t: string) {
    text = t
    reanalyze(t)
    clearTimeout(draftTimer)
    draftTimer = setTimeout(saveDraft, 400)
  }

  function saveDraft() {
    store(DRAFT_KEY, { text, path: filePath, name: fileName } satisfies Draft)
  }

  function onCursor(pos: number) {
    cursor = pos
    if (dragging || !current()) return
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
    if (!current()) return
    const path = analysis.paths[pi]
    if (j >= 0) editor.select(path.points[j].node[0], path.points[j].node[0] + 5)
    else editor.select(path.name[0], path.name[1])
    activePath = pi
    activePoint = j
  }

  function onDragStart(pi: number, j: number) {
    if (!current()) return
    dragging = { path: pi, point: analysis.paths[pi].points[j] }
    editor.beginTransient()
  }

  function onDrag(x: number, y: number, decimals: number) {
    if (!dragging) return
    editor.transient(movePoint(dragging.point, x, y, decimals))
  }

  function onDragEnd() {
    if (!dragging) return
    editor.commitTransient()
    dragging = null
    clearTimeout(fullTimer)
    if (analysis.ok) renderer.render(text, 1).then(show)
  }

  function decimalsOf(p: PointInfo) {
    return Number.isInteger(p.x) && Number.isInteger(p.y) ? 0 : 1
  }

  function onNudge(dx: number, dy: number) {
    if (!current()) return
    const p = analysis.paths[activePath]?.points[activePoint]
    if (!p) return
    editor.apply(movePoint(p, p.x + dx, p.y + dy, decimalsOf(p)), p.node[0], p.node[0] + 5)
  }

  function onInsert(pi: number, after: number, x: number, y: number) {
    if (!current()) return
    const change = insertPoint(text, analysis.paths[pi], after, Math.round(x), Math.round(y))
    editor.apply([change])
    const at = change.from + change.insert.indexOf('point')
    editor.select(at, at + 5)
  }

  function onDelete(pi: number, j: number) {
    if (!current()) return
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

  async function formatScene() {
    const src = text
    let formatted: string
    try {
      formatted = await backend.format(src)
    } catch (e) {
      return flash(`Cannot format: ${errorText(e)}`)
    }
    if (text !== src) return // Edited while formatting; the result is stale.
    if (formatted === src) return flash('Already formatted')
    editor.apply(layoutChanges(src, formatted) ?? [{ from: 0, to: src.length, insert: formatted }])
  }

  // --- Files

  const KDL_FILTER = [{ name: 'szeel scene', extensions: ['kdl'] }]

  function flash(msg: string) {
    toast = msg
    setTimeout(() => toast === msg && (toast = null), 2500)
  }

  function errorText(e: unknown) {
    return e instanceof Error ? e.message : String(e)
  }

  /**
   * Puts `t` in the editor as the scene at `path` (or an untitled one named
   * `name`). `saved` is the text on disk, if different from `t`. `fit`
   * fits the scene to the view.
   */
  function loadText(t: string, path: string | null, name = 'untitled.kdl', saved: string | null = t, fit = true) {
    filePath = path
    untitledName = name
    savedText = saved
    lastFullMs = 0
    activePath = activePoint = -1
    editor.load(t)
    if (fit) queueMicrotask(() => preview.fit())
    backend.watch(path).catch((e) => flash(`Cannot watch ${baseName(path!)}: ${errorText(e)}`))
    if (path) remember(path)
    saveDraft()
  }

  function remember(path: string) {
    recent = [path, ...recent.filter((p) => p !== path)].slice(0, MAX_RECENT)
    store(RECENT_KEY, recent)
  }

  function forget(path: string) {
    recent = recent.filter((p) => p !== path)
    store(RECENT_KEY, recent)
  }

  /** Asks what to do with unsaved changes. True if it is fine to go on and drop the text. */
  async function settle(): Promise<boolean> {
    if (!dirty) return true
    const answer = await message(`Save the changes to ${fileName}?`, {
      title: 'Unsaved changes',
      kind: 'warning',
      buttons: { yes: 'Save', no: 'Discard', cancel: 'Cancel' },
    })
    if (answer === 'Save' || answer === 'Yes') return saveFile()
    return answer === 'Discard' || answer === 'No'
  }

  async function newScene() {
    if (await settle()) loadText(BLANK, null)
  }

  async function openExample(name: string) {
    if (await settle()) loadText(examples[name], null, name)
  }

  async function openPath(path: string) {
    if (path === filePath) return
    if (!(await settle())) return
    try {
      loadText(await backend.readText(path), path)
    } catch (e) {
      forget(path)
      flash(`Cannot open ${baseName(path)}: ${errorText(e)}`)
    }
  }

  async function openFile() {
    const path = await open({
      filters: KDL_FILTER,
      defaultPath: filePath ? dirName(filePath) : recent[0] ? dirName(recent[0]) : undefined,
    })
    if (path) openPath(path)
  }

  /** Saves to `path`, or to the open file. True if saved. */
  async function writeTo(path: string): Promise<boolean> {
    const snapshot = text
    try {
      await backend.writeText(path, snapshot)
    } catch (e) {
      flash(`Cannot save ${baseName(path)}: ${errorText(e)}`)
      return false
    }
    savedText = snapshot
    if (path !== filePath) {
      filePath = path
      backend.watch(path).catch(() => {})
    }
    remember(path)
    saveDraft()
    flash(`Saved ${baseName(path)}`)
    return true
  }

  async function saveFileAs(): Promise<boolean> {
    const path = await save({
      filters: KDL_FILTER,
      defaultPath: filePath ?? (recent[0] ? `${dirName(recent[0])}/${untitledName}` : untitledName),
    })
    return path ? writeTo(path) : false
  }

  function saveFile(): Promise<boolean> {
    return filePath ? writeTo(filePath) : saveFileAs()
  }

  async function exportPng() {
    if (!analysis.ok) return flash('Fix the errors first')
    const stem = fileName.replace(/\.kdl$/i, '') + (exportScale !== 1 ? `@${exportScale}x` : '')
    const path = await save({
      filters: [{ name: 'PNG image', extensions: ['png'] }],
      defaultPath: filePath ? `${dirName(filePath)}/${stem}.png` : `${stem}.png`,
    })
    if (!path) return
    flash(`Exporting ${baseName(path)}…`)
    try {
      await backend.exportPng(text, exportScale, path)
      flash(`Exported ${baseName(path)}`)
    } catch (e) {
      flash(`Cannot export: ${errorText(e)}`)
    }
  }

  // The open file changed on disk: take the new text if nothing here is unsaved, or ask.
  let changeTimer: ReturnType<typeof setTimeout> | undefined
  let asking = false

  function onFileChanged(path: string) {
    if (path !== filePath) return
    clearTimeout(changeTimer)
    changeTimer = setTimeout(() => reloadFromDisk(path), 150)
  }

  async function reloadFromDisk(path: string) {
    if (asking || path !== filePath) return
    let disk: string
    try {
      disk = await backend.readText(path)
    } catch {
      return // Mid-replace or deleted; a later event or the next save sorts it out.
    }
    if (disk === savedText || path !== filePath) return
    if (dirty && disk !== text) {
      asking = true
      const reload = await message(`${baseName(path)} was changed by another program. Reload it and lose your changes here?`, {
        title: 'File changed on disk',
        kind: 'warning',
        buttons: { ok: 'Reload', cancel: 'Keep mine' },
      })
      asking = false
      if (reload !== 'Reload' && reload !== 'Ok') {
        savedText = disk // Don't ask again about this version; saving overwrites it.
        return
      }
    }
    if (disk === text) {
      savedText = disk
      return
    }
    loadText(disk, path, undefined, disk, false)
    flash(`Reloaded ${baseName(path)}`)
  }

  /** Picks the scene to start with: the file on the command line, else the last session's draft. */
  async function start() {
    const arg = await backend.initialFile()
    if (arg) {
      try {
        return loadText(await backend.readText(arg), arg)
      } catch (e) {
        flash(`Cannot open ${baseName(arg)}: ${errorText(e)}`)
      }
    }
    const draft = stored<Draft>(DRAFT_KEY)
    if (draft && typeof draft.text === 'string') {
      if (draft.path) {
        try {
          return loadText(draft.text, draft.path, undefined, await backend.readText(draft.path))
        } catch {
          return loadText(draft.text, null, baseName(draft.path), null)
        }
      }
      const name = String(draft.name ?? 'untitled.kdl')
      // An untouched example or blank scene isn't unsaved work.
      const pristine = draft.text === examples[name] || draft.text === BLANK
      return loadText(draft.text, null, name, pristine ? draft.text : null)
    }
    loadText(examples['rainbow.kdl'] ?? BLANK, null, 'rainbow.kdl')
  }

  // --- Layout

  function readSplit() {
    const v = Number(stored<number>(SPLIT_KEY))
    return v > 0.1 && v < 0.9 ? v : 0.4
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
      store(SPLIT_KEY, split)
    }
    el.addEventListener('pointermove', move)
    el.addEventListener('pointerup', up)
  }

  // --- Menu

  let menu = $state.raw<AppMenu>()
  const canExport = $derived(analysis.ok)
  let canUndo = $state(false)
  let canRedo = $state(false)

  $effect(() => {
    menu?.update({ recent, canExport, canUndo, canRedo, exportScale, showOverlay, showHidden })
  })

  async function setUpMenu() {
    menu = await installMenu(
      {
        newScene,
        openFile,
        openRecent: openPath,
        openExample,
        saveFile,
        saveFileAs,
        exportPng,
        setExportScale: (s) => (exportScale = s),
        quit: () => getCurrentWindow().close(),
        undo: () => editor.undo(),
        redo: () => editor.redo(),
        formatScene,
        addPath,
        setOverlay: (on) => (showOverlay = on),
        setHidden: (on) => (showHidden = on),
        zoomIn: () => preview.zoomBy(1.25),
        zoomOut: () => preview.zoomBy(1 / 1.25),
        fit: () => preview.fit(),
        actualSize: () => preview.actualSize(),
      },
      Object.keys(examples),
      (path) => `${baseName(path)} — ${dirName(path)}`,
    )
  }

  onMount(() => {
    const win = getCurrentWindow()
    const unlisten = [
      listen<string>('file-changed', (e) => onFileChanged(e.payload)),
      win.onCloseRequested(async (e) => {
        if (!(await settle())) e.preventDefault()
      }),
      getCurrentWebview().onDragDropEvent((e) => {
        if (e.payload.type !== 'drop') return
        const path = e.payload.paths.find((p) => /\.kdl$/i.test(p))
        if (path) openPath(path)
        else if (e.payload.paths.length) flash('Drop a .kdl file to open it')
      }),
    ]
    start()
    setUpMenu().catch((e) => flash(`Cannot set up the menu: ${errorText(e)}`))
    return () => unlisten.forEach((u) => u.then((f) => f()))
  })
</script>

<main bind:this={main} style:grid-template-columns="{split * 100}% 6px 1fr">
  <section class="left">
    <div class="code">
      <Editor
        bind:this={editor}
        doc={text}
        diagnostics={analysis.diagnostics}
        onchange={onTextChange}
        oncursor={onCursor}
        onhistory={(u, r) => ((canUndo = u), (canRedo = r))}
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
    {#if toast}<div class="toast">{toast}</div>{/if}
  </section>
</main>

<style>
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
