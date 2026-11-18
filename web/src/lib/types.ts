/** A source range as UTF-16 offsets: [from, to). */
export type Range = [number, number]

export interface Diagnostic {
  severity: 'error' | 'warning' | 'info'
  message: string
  label: string | null
  help: string | null
  range: Range
}

export interface PointInfo {
  x: number
  y: number
  /** The whole `node` or `cusp` node. */
  node: Range
  xr: Range
  yr: Range
  /** Whether the curve may turn sharply here (a `cusp`, not a `node`). */
  cusp: boolean
  /** The node name, `node` or `cusp`. */
  nr: Range
}

export type Role = 'draw' | 'mask' | 'defs'

export interface PathInfo {
  node: Range
  name: Range
  id: string | null
  closed: boolean
  role: Role
  points: PointInfo[]
  /** Bézier segments: x0 y0 x1 y1 x2 y2 x3 y3. */
  curves: number[][]
}

export interface Analysis {
  ok: boolean
  width: number
  height: number
  diagnostics: Diagnostic[]
  paths: PathInfo[]
}
