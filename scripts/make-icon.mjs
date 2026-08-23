/**
 * Renders the ztorrent app icon to PNG with no external image tooling.
 *
 * The mark is a Z in five horizontal lines: a long bar, three short steps
 * walking down the diagonal, a long bar. The steps are drawn lighter than the
 * bars, so the bars carry the letter and the steps read as what is still
 * arriving. Flat ink, no gradient. It is drawn in the 24-unit grid the line
 * icons in renderer/icons.js use, so the dock icon and the toolbar are one
 * family.
 *
 *   node scripts/make-icon.mjs            # writes build/icon.png (1024²)
 *   ./scripts/make-icns.sh                # turns that into build/icon.icns
 */
import fs from 'node:fs'
import path from 'node:path'
import zlib from 'node:zlib'
import { fileURLToPath } from 'node:url'

const SIZE = 1024
const SS = 3                       // supersampling factor for anti-aliasing
const BOX = 824                    // macOS content square inside the 1024 canvas
const RADIUS = 185

// ---- the mark, in the 24-unit grid the UI icons are drawn in ---------------

const LINES = 5                    // total lines: two bars plus the steps between
const BAR_H = 3.4                  // the two long bars' thickness
const STEP_H = 2.0                 // the steps run lighter than the bars
const BAR = [5.8, 18.2]            // the long top and bottom bars, left to right
const STEP = 4.0                   // length of the lines that step down the diagonal
const GRID_H = 17                  // the mark's ink height, top edge to bottom edge
const TOP_Y = 3.5                  // the mark's top edge
const MARK_H = 0.56                // mark height as a fraction of the content box

const TILE = [0x1f, 0x22, 0x26]    // flat graphite — no gradient
const INK = [0xff, 0xff, 0xff]
// A hairline lift along the tile edge. Near-black would otherwise lose its
// silhouette against a dark desktop; this is the whole of the relief, and it
// vanishes by the time the icon is 16px.
const RIM = BOX * 0.0097
const RIM_LIFT = 26

const isBar = i => i === 0 || i === LINES - 1
const THICK = Array.from({ length: LINES }, (_, i) => (isBar(i) ? BAR_H : STEP_H))

// The lines are laid out from the top edge down; whatever height the strokes
// leave over is split evenly into the gaps between them.
const LINE_GAP = (GRID_H - THICK.reduce((a, b) => a + b, 0)) / (LINES - 1)

/**
 * The lines as [x1, y, x2, y, thickness] in grid units. The outer two are the
 * Z's bars; the ones between are short steps walking down the diagonal.
 *
 * Every gap in the descent is the same: bar to first step, step to step, and
 * last step to bar all move left by SHIFT. That makes SHIFT a consequence of
 * the step length rather than a free choice — the equal gaps have to divide
 * what the bars leave over — so shortening STEP widens the stagger, and
 * dropping a line widens it again.
 */
const SHIFT = (BAR[1] - BAR[0] - STEP) / (LINES - 1)
const PIECES = (() => {
  let top = TOP_Y
  return THICK.map((h, i) => {
    const y = top + h / 2
    top += h + LINE_GAP
    if (isBar(i)) return [BAR[0], y, BAR[1], y, h]
    const right = BAR[1] - i * SHIFT
    return [right - STEP, y, right, y, h]
  })
})()
const K = (BOX * MARK_H) / GRID_H          // grid units to canvas pixels
const to = v => SIZE / 2 + (v - 12) * K    // grid coordinate to canvas pixel

/** Signed distance to an axis-aligned rounded rect: <= 0 means inside. */
function roundRect (x, y, cx, cy, hw, hh, r) {
  const dx = Math.abs(x - cx) - (hw - r)
  const dy = Math.abs(y - cy) - (hh - r)
  return Math.hypot(Math.max(dx, 0), Math.max(dy, 0)) + Math.min(Math.max(dx, dy), 0) - r
}

const tileDist = (x, y) => roundRect(x, y, SIZE / 2, SIZE / 2, BOX / 2, BOX / 2, RADIUS)

const PIECES_PX = PIECES.map(([x1, y1, x2, y2, h]) => [to(x1), to(y1), to(x2), to(y2), (h / 2) * K])

/** Distance from a point to a segment. */
function segDist (px, py, ax, ay, bx, by) {
  const vx = bx - ax
  const vy = by - ay
  const len2 = vx * vx + vy * vy
  let t = len2 ? ((px - ax) * vx + (py - ay) * vy) / len2 : 0
  t = t < 0 ? 0 : t > 1 ? 1 : t
  return Math.hypot(px - (ax + t * vx), py - (ay + t * vy))
}

/** <= 0 inside the mark: the union of the lines, each a round-capped bar. */
function markDist (x, y) {
  let d = Infinity
  for (const [ax, ay, bx, by, half] of PIECES_PX) {
    d = Math.min(d, segDist(x, y, ax, ay, bx, by) - half)
  }
  return d
}

const px = new Uint8Array(SIZE * SIZE * 4)

for (let y = 0; y < SIZE; y++) {
  for (let x = 0; x < SIZE; x++) {
    let rs = 0; let gs = 0; let bs = 0; let as = 0
    for (let sy = 0; sy < SS; sy++) {
      for (let sx = 0; sx < SS; sx++) {
        const fx = x + (sx + 0.5) / SS
        const fy = y + (sy + 0.5) / SS
        const td = tileDist(fx, fy)
        if (td > 0) continue
        const edge = td > -RIM
        const [r, g, b] = markDist(fx, fy) <= 0
          ? INK
          : edge ? TILE.map(v => Math.min(255, v + RIM_LIFT)) : TILE
        rs += r; gs += g; bs += b; as += 255
      }
    }
    const n = SS * SS
    const i = (y * SIZE + x) * 4
    const cover = as / (255 * n)
    px[i] = cover ? Math.round(rs / (n * cover)) : 0
    px[i + 1] = cover ? Math.round(gs / (n * cover)) : 0
    px[i + 2] = cover ? Math.round(bs / (n * cover)) : 0
    px[i + 3] = Math.round(as / n)
  }
}

// ---- minimal PNG writer ----------------------------------------------------

const CRC_TABLE = (() => {
  const t = new Uint32Array(256)
  for (let n = 0; n < 256; n++) {
    let c = n
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1
    t[n] = c >>> 0
  }
  return t
})()

function crc32 (buf) {
  let c = 0xffffffff
  for (const b of buf) c = CRC_TABLE[(c ^ b) & 0xff] ^ (c >>> 8)
  return (c ^ 0xffffffff) >>> 0
}

function chunk (type, data) {
  const len = Buffer.alloc(4)
  len.writeUInt32BE(data.length)
  const body = Buffer.concat([Buffer.from(type, 'ascii'), data])
  const crc = Buffer.alloc(4)
  crc.writeUInt32BE(crc32(body))
  return Buffer.concat([len, body, crc])
}

const ihdr = Buffer.alloc(13)
ihdr.writeUInt32BE(SIZE, 0)
ihdr.writeUInt32BE(SIZE, 4)
ihdr[8] = 8      // bit depth
ihdr[9] = 6      // RGBA
ihdr[10] = 0; ihdr[11] = 0; ihdr[12] = 0

const raw = Buffer.alloc(SIZE * (SIZE * 4 + 1))
for (let y = 0; y < SIZE; y++) {
  raw[y * (SIZE * 4 + 1)] = 0     // filter: none
  Buffer.from(px.buffer, y * SIZE * 4, SIZE * 4).copy(raw, y * (SIZE * 4 + 1) + 1)
}

const png = Buffer.concat([
  Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
  chunk('IHDR', ihdr),
  chunk('IDAT', zlib.deflateSync(raw, { level: 9 })),
  chunk('IEND', Buffer.alloc(0))
])

const BUILD = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', 'build')
fs.mkdirSync(BUILD, { recursive: true })

const out = path.join(BUILD, 'icon.png')
fs.writeFileSync(out, png)
console.log(`wrote ${out} (${SIZE}x${SIZE}, ${(png.length / 1024).toFixed(1)} kB)`)

// ---- the same mark as vector, for READMEs and anywhere else it is needed ---

const round = v => +v.toFixed(2)
const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="24" height="24"
     fill="none" stroke="currentColor" stroke-linecap="round">
  <title>ztorrent</title>
${PIECES.map(([x1, y1, x2, y2, h]) =>
  `  <path d="M${round(x1)} ${round(y1)}L${round(x2)} ${round(y2)}" stroke-width="${round(h)}"/>`
).join('\n')}
</svg>
`
const svgOut = path.join(BUILD, 'logo.svg')
fs.writeFileSync(svgOut, svg)
console.log(`wrote ${svgOut}`)
