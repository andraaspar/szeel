import type { PathInfo, PointInfo } from './types'

/** One text replacement, in the coordinates of the text it applies to. */
export interface Change {
  from: number
  to: number
  insert: string
}

/** Formats a coordinate with at most `decimals` decimals and no trailing zeros. */
export function fmt(v: number, decimals = 0): string {
  const s = v.toFixed(decimals)
  const n = Number(s)
  return Object.is(n, -0) ? '0' : String(n)
}

export function movePoint(p: PointInfo, x: number, y: number, decimals = 0): Change[] {
  return [
    { from: p.xr[0], to: p.xr[1], insert: fmt(x, decimals) },
    { from: p.yr[0], to: p.yr[1], insert: fmt(y, decimals) },
  ]
}

/** Turns a `node` into a `cusp`, or a `cusp` back into a smooth `node`. */
export function toggleCusp(p: PointInfo): Change {
  return { from: p.nr[0], to: p.nr[1], insert: p.cusp ? 'node' : 'cusp' }
}

function lineStart(text: string, i: number): number {
  return text.lastIndexOf('\n', i - 1) + 1
}

function lineEnd(text: string, i: number): number {
  const e = text.indexOf('\n', i)
  return e < 0 ? text.length : e
}

/** End of a node's span with trailing whitespace and terminator removed. */
function trimmedEnd(text: string, [from, to]: [number, number]): number {
  let e = to
  while (e > from && /[\s;]/.test(text[e - 1])) e--
  return e
}

/** Inserts a point after point `after` of `path` (or before the first if -1). */
export function insertPoint(text: string, path: PathInfo, after: number, x: number, y: number): Change {
  const node = `node ${fmt(x)} ${fmt(y)}`
  const anchor = path.points[Math.max(after, 0)]
  if (after < 0) {
    const ls = lineStart(text, anchor.node[0])
    const indent = text.slice(ls, anchor.node[0])
    if (/^\s*$/.test(indent)) return { from: ls, to: ls, insert: `${indent}${node}\n` }
    return { from: anchor.node[0], to: anchor.node[0], insert: `${node}; ` }
  }
  const e = trimmedEnd(text, anchor.node)
  const ls = lineStart(text, anchor.node[0])
  const le = lineEnd(text, e)
  const before = text.slice(ls, anchor.node[0])
  const rest = text.slice(e, le)
  if (/^\s*$/.test(before) && /^\s*;?\s*(\/\/.*)?$/.test(rest)) {
    return { from: le, to: le, insert: `\n${before}${node}` }
  }
  return { from: e, to: e, insert: `; ${node}` }
}

/** Removes a `node` or `cusp`, and its line if it stands alone on it. */
export function deletePoint(text: string, p: PointInfo): Change {
  const e = trimmedEnd(text, p.node)
  const ls = lineStart(text, p.node[0])
  const le = lineEnd(text, e)
  if (/^\s*$/.test(text.slice(ls, p.node[0])) && /^\s*;?\s*$/.test(text.slice(e, le))) {
    return { from: ls, to: Math.min(le + 1, text.length), insert: '' }
  }
  const m = /^\s*;\s*/.exec(text.slice(e))
  return { from: p.node[0], to: e + (m ? m[0].length : 0), insert: '' }
}

/**
 * Adds a new path at the end of the file, so it is drawn in front of
 * everything else, centred on (cx, cy).
 */
export function newPath(text: string, cx: number, cy: number, size: number): Change {
  const at = text.length
  const r = Math.max(10, Math.round(size / 4))
  const hue = Math.floor(Math.random() * 360)
  const insert =
    `\npath {\n` +
    `    node ${fmt(cx - r)} ${fmt(cy)}\n` +
    `    node ${fmt(cx)} ${fmt(cy - r / 2)}\n` +
    `    node ${fmt(cx + r)} ${fmt(cy)}\n` +
    `    gradient 1.5 {\n` +
    `        stop 0 0.7 0.17 ${hue}\n` +
    `        stop ${Math.round(r / 5)} 0.7 0.17 ${hue} 0\n` +
    `    }\n` +
    `}\n`
  return { from: at, to: at, insert: text && !text.endsWith('\n') ? '\n' + insert : insert }
}

/** Index of the innermost path whose node contains `pos`, or -1. */
export function pathAt(paths: PathInfo[], pos: number): number {
  let best = -1
  for (let i = 0; i < paths.length; i++) {
    const [a, b] = paths[i].node
    if (pos >= a && pos <= b && (best < 0 || b - a < paths[best].node[1] - paths[best].node[0])) best = i
  }
  return best
}

/**
 * For each Bézier segment of a path, the index of the point it starts from.
 * Segments are built from distinct points only, so repeated points are skipped.
 */
export function segmentStarts(path: PathInfo): number[] {
  const starts: number[] = []
  let prev: PointInfo | null = null
  path.points.forEach((p, i) => {
    if (!prev || prev.x !== p.x || prev.y !== p.y) starts.push(i)
    prev = p
  })
  return starts
}

/** Characters a reformat may add, drop or change. */
const LAYOUT = /[\s;]/

/**
 * The changes that turn `from` into `to` when they differ only in layout
 * (whitespace and `;`), so the cursor and undo history stay where they
 * belong. Null if they differ in anything else.
 */
export function layoutChanges(from: string, to: string): Change[] | null {
  const changes: Change[] = []
  let i = 0
  let j = 0
  for (;;) {
    let i2 = i
    let j2 = j
    while (i2 < from.length && LAYOUT.test(from[i2])) i2++
    while (j2 < to.length && LAYOUT.test(to[j2])) j2++
    if (from.slice(i, i2) !== to.slice(j, j2)) changes.push({ from: i, to: i2, insert: to.slice(j, j2) })
    if (i2 === from.length || j2 === to.length) return i2 === from.length && j2 === to.length ? changes : null
    if (from[i2] !== to[j2]) return null
    i = i2 + 1
    j = j2 + 1
  }
}
