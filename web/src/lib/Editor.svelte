<script lang="ts">
  import { onMount } from 'svelte'
  import { ChangeSet, EditorState, Text, Transaction } from '@codemirror/state'
  import type { Extension } from '@codemirror/state'
  import {
    EditorView,
    drawSelection,
    highlightActiveLine,
    highlightActiveLineGutter,
    highlightSpecialChars,
    keymap,
    lineNumbers,
  } from '@codemirror/view'
  import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands'
  import { bracketMatching, foldGutter, foldKeymap, indentOnInput, indentUnit } from '@codemirror/language'
  import { closeBrackets, closeBracketsKeymap } from '@codemirror/autocomplete'
  import { highlightSelectionMatches, searchKeymap } from '@codemirror/search'
  import { lintGutter, setDiagnostics } from '@codemirror/lint'
  import { kdlHighlight, kdlLanguage } from './kdl-lang'
  import type { Change } from './edits'
  import type { Diagnostic } from './types'

  interface Props {
    doc: string
    diagnostics: Diagnostic[]
    onchange: (text: string) => void
    oncursor: (pos: number) => void
  }
  let { doc, diagnostics, onchange, oncursor }: Props = $props()

  let host: HTMLDivElement
  let view: EditorView | undefined

  const theme = EditorView.theme({
    '&': { height: '100%', fontSize: '13px', backgroundColor: 'var(--panel)', color: 'var(--text)' },
    '.cm-scroller': { fontFamily: 'var(--mono)', lineHeight: '1.55' },
    '.cm-content': { caretColor: 'var(--accent)' },
    '.cm-cursor': { borderLeftColor: 'var(--accent)', borderLeftWidth: '2px' },
    '.cm-gutters': { backgroundColor: 'var(--panel)', color: 'var(--muted)', border: 'none' },
    '.cm-activeLine': { backgroundColor: 'var(--active-line)' },
    '.cm-activeLineGutter': { backgroundColor: 'var(--active-line)', color: 'var(--text)' },
    '&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection': {
      backgroundColor: 'var(--selection) !important',
    },
    '.cm-selectionMatch': { backgroundColor: 'var(--selection-match)' },
    '.cm-tooltip': { backgroundColor: 'var(--panel-2)', border: '1px solid var(--border)', color: 'var(--text)' },
    '.cm-diagnostic': { fontFamily: 'var(--sans)' },
    '.cm-panels': { backgroundColor: 'var(--panel-2)', color: 'var(--text)' },
  })

  let extensions: Extension[] = []

  onMount(() => {
    extensions = [
          lineNumbers(),
          highlightActiveLineGutter(),
          foldGutter(),
          lintGutter(),
          highlightSpecialChars(),
          history(),
          drawSelection(),
          indentOnInput(),
          indentUnit.of('    '),
          bracketMatching(),
          closeBrackets(),
          highlightActiveLine(),
          highlightSelectionMatches(),
          kdlLanguage,
          kdlHighlight,
          theme,
          keymap.of([
            ...closeBracketsKeymap,
            ...defaultKeymap,
            ...searchKeymap,
            ...historyKeymap,
            ...foldKeymap,
            indentWithTab,
          ]),
          EditorView.updateListener.of((u) => {
            if (u.docChanged) onchange(u.state.doc.toString())
            if (u.selectionSet || u.docChanged) oncursor(u.state.selection.main.head)
          }),
        ]
    view = new EditorView({ parent: host, state: EditorState.create({ doc, extensions }) })
    return () => view?.destroy()
  })

  $effect(() => {
    if (!view) return
    const len = view.state.doc.length
    view.dispatch(
      setDiagnostics(
        view.state,
        diagnostics.map((d) => ({
          from: Math.min(d.range[0], len),
          to: Math.min(Math.max(d.range[1], d.range[0]), len),
          severity: d.severity,
          message: d.message + (d.label ? ` (${d.label})` : '') + (d.help ? `\n${d.help}` : ''),
        })),
      ),
    )
  })

  function sorted(changes: Change[]) {
    return [...changes].sort((a, b) => a.from - b.from)
  }

  /** Replaces the whole document, starting a fresh undo history. */
  export function load(text: string) {
    if (!view) return
    view.setState(EditorState.create({ doc: text, extensions }))
    onchange(text)
    oncursor(0)
  }

  export function apply(changes: Change[], selectFrom?: number, selectTo?: number) {
    if (!view) return
    const set = ChangeSet.of(sorted(changes), view.state.doc.length)
    view.dispatch({
      changes: set,
      selection: selectFrom === undefined ? undefined : { anchor: set.mapPos(selectFrom, 1), head: set.mapPos(selectTo ?? selectFrom, 1) },
      scrollIntoView: selectFrom !== undefined,
      userEvent: 'input',
    })
  }

  export function select(from: number, to = from) {
    if (!view) return
    const len = view.state.doc.length
    view.dispatch({ selection: { anchor: Math.min(from, len), head: Math.min(to, len) }, scrollIntoView: true })
  }

  export function focus() {
    view?.focus()
  }

  // --- Transient edits: a drag rewrites the text many times but is undone in one step.

  let base: Text | null = null
  let prev: ChangeSet | null = null

  /** Starts a transient edit; later `transient` changes are relative to the text as it is now. */
  export function beginTransient() {
    if (!view) return
    base = view.state.doc
    prev = ChangeSet.empty(base.length)
  }

  export function transient(changes: Change[]) {
    if (!view || !base || !prev) return
    const next = ChangeSet.of(sorted(changes), base.length)
    view.dispatch({
      changes: prev.invert(base).compose(next),
      annotations: Transaction.addToHistory.of(false),
    })
    prev = next
  }

  /** Ends a transient edit, recording its net change as a single undo step. */
  export function commitTransient() {
    if (!view || !base || !prev) return
    const [b, p] = [base, prev]
    base = prev = null
    if (p.empty) return
    view.dispatch({ changes: p.invert(b), annotations: Transaction.addToHistory.of(false) })
    view.dispatch({ changes: p, userEvent: 'input.drag' })
  }
</script>

<div class="editor" bind:this={host}></div>

<style>
  .editor {
    height: 100%;
    min-height: 0;
    overflow: hidden;
  }
</style>
