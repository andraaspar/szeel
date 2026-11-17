import { HighlightStyle, StreamLanguage, syntaxHighlighting } from '@codemirror/language'
import type { StringStream } from '@codemirror/language'
import { tags as t } from '@lezer/highlight'

/** Node names szeel understands; anything else is shown as a plain name. */
const KNOWN = new Set(['image', 'path', 'point', 'gradient', 'stop', 'group', 'mask', 'use', 'defs'])

interface State {
  /** Next identifier starts a node. */
  atNode: boolean
  /** Inside a block comment, at this nesting depth. */
  comment: number
  /** Inside a multi-line string. */
  string: false | 'plain' | { hashes: string }
}

const IDENT = /^[^\s\\/(){}[\];="#]+/

function readString(stream: StringStream, state: State): string {
  const s = state.string
  if (s === 'plain') {
    while (!stream.eol()) {
      const c = stream.next()
      if (c === '\\') stream.next()
      else if (c === '"') {
        state.string = false
        return 'string'
      }
    }
    return 'string'
  }
  if (s && typeof s === 'object') {
    const end = '"' + s.hashes
    while (!stream.eol()) {
      if (stream.match(end)) {
        state.string = false
        return 'string'
      }
      stream.next()
    }
  }
  return 'string'
}

/** A small tokenizer for KDL v2, enough for highlighting. */
export const kdlLanguage = StreamLanguage.define<State>({
  name: 'kdl',
  startState: () => ({ atNode: true, comment: 0, string: false }),
  token(stream, state) {
    if (state.comment > 0) {
      while (!stream.eol()) {
        if (stream.match('/*')) state.comment++
        else if (stream.match('*/')) {
          if (--state.comment === 0) break
        } else stream.next()
      }
      return 'comment'
    }
    if (state.string) return readString(stream, state)

    if (stream.sol() && state.atNode === false) {
      // A line break ends a node unless it was escaped; KDL's `\` continuation is rare here.
      state.atNode = true
    }
    if (stream.eatSpace()) return null
    if (stream.match('//')) {
      stream.skipToEnd()
      return 'comment'
    }
    if (stream.match('/*')) {
      state.comment = 1
      return 'comment'
    }
    if (stream.match('/-')) return 'comment'

    const c = stream.peek()!
    if (c === '{' || c === ';') {
      stream.next()
      state.atNode = true
      return 'brace'
    }
    if (c === '}') {
      stream.next()
      state.atNode = true
      return 'brace'
    }
    if (c === '=') {
      stream.next()
      return 'operator'
    }
    if (c === '(') {
      stream.match(/^\([^)]*\)?/)
      return 'typeName'
    }
    if (stream.match(/^#+"/)) {
      const hashes = stream.current().slice(0, -1)
      state.string = { hashes }
      return readString(stream, state)
    }
    if (c === '"') {
      stream.next()
      state.string = 'plain'
      return readString(stream, state)
    }
    if (stream.match(/^#(true|false|null|inf|-inf|nan)\b/)) return 'atom'
    if (stream.match(/^[+-]?(0x[0-9a-fA-F_]+|0o[0-7_]+|0b[01_]+|\d[\d_]*(\.\d[\d_]*)?([eE][+-]?\d+)?)(?![^\s;{}()=\\])/)) {
      return 'number'
    }
    if (stream.match(IDENT)) {
      const word = stream.current()
      if (state.atNode) {
        state.atNode = false
        return KNOWN.has(word) ? 'keyword' : 'variableName'
      }
      if (stream.peek() === '=') return 'propertyName'
      return 'string'
    }
    stream.next()
    return null
  },
  languageData: { commentTokens: { line: '//', block: { open: '/*', close: '*/' } } },
})

export const kdlHighlight = syntaxHighlighting(
  HighlightStyle.define([
    { tag: t.keyword, color: 'var(--syn-keyword)', fontWeight: '600' },
    { tag: t.variableName, color: 'var(--syn-name)' },
    { tag: t.propertyName, color: 'var(--syn-prop)' },
    { tag: t.number, color: 'var(--syn-number)' },
    { tag: t.string, color: 'var(--syn-string)' },
    { tag: t.atom, color: 'var(--syn-atom)' },
    { tag: t.comment, color: 'var(--syn-comment)', fontStyle: 'italic' },
    { tag: t.typeName, color: 'var(--syn-type)' },
    { tag: [t.brace, t.operator], color: 'var(--syn-punct)' },
  ]),
)
