/**
 * Renders the ztorrent app icon to PNG with no external image tooling.
 *
 * The mark is a geometric Z whose diagonal doubles as the downward stroke of a
 * download: it names the app and says what it does in one shape, and it stays
 * legible at 16px in a dock or task bar.
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
const OFF = (SIZE - BOX) / 2

// The glyph, in the 16-unit space the UI icons are drawn in.
const UNIT = BOX / 16
const STROKE = 1.85 * UNIT / 2
const pt = (x, y) => [OFF + x * UNIT, OFF + y * UNIT]
// A Z drawn as three strokes: top bar, diagonal, bottom bar. Round caps and
// joins fall out of the distance field for free, matching the round-capped
// line icons used throughout the UI.
const SEGMENTS = [
  [pt(4.45, 4.55), pt(11.55, 4.55)],
  [pt(11.55, 4.55), pt(4.45, 11.45)],
  [pt(4.45, 11.45), pt(11.55, 11.45)]
]

const clamp01 = v => (v < 0 ? 0 : v > 1 ? 1 : v)

/** Signed-ish distance to a rounded rectangle: <= 0 means inside. */
function roundedRectDist (x, y) {
  const cx = SIZE / 2
  const cy = SIZE / 2
  const hw = BOX / 2 - RADIUS
  const hh = BOX / 2 - RADIUS
  const dx = Math.max(Math.abs(x - cx) - hw, 0)
  const dy = Math.max(Math.abs(y - cy) - hh, 0)
  return Math.hypot(dx, dy) - RADIUS
}

function segDist (px, py, [ax, ay], [bx, by]) {
  const vx = bx - ax
  const vy = by - ay
  const wx = px - ax
  const wy = py - ay
  const len2 = vx * vx + vy * vy
  const t = len2 ? clamp01((wx * vx + wy * vy) / len2) : 0
  return Math.hypot(px - (ax + t * vx), py - (ay + t * vy))
}

const glyphDist = (x, y) => Math.min(...SEGMENTS.map(s => segDist(x, y, s[0], s[1])))



// Vertical gradient in the family of the UI accent, deepened so the white
// mark carries plenty of contrast at every size.
const TOP = [0x6a, 0x9b, 0xff]
const MID = [0x3b, 0x6f, 0xe8]
const BOT = [0x24, 0x3d, 0xa8]
function gradient (t) {
  const [a, b, u] = t < 0.55
    ? [TOP, MID, t / 0.55]
    : [MID, BOT, (t - 0.55) / 0.45]
  return [0, 1, 2].map(i => Math.round(a[i] + (b[i] - a[i]) * u))
}

const px = new Uint8Array(SIZE * SIZE * 4)

for (let y = 0; y < SIZE; y++) {
  for (let x = 0; x < SIZE; x++) {
    let rs = 0; let gs = 0; let bs = 0; let as = 0
    for (let sy = 0; sy < SS; sy++) {
      for (let sx = 0; sx < SS; sx++) {
        const fx = x + (sx + 0.5) / SS
        const fy = y + (sy + 0.5) / SS
        if (roundedRectDist(fx, fy) > 0) continue

        const [r, g, b] = gradient((fy - OFF) / BOX)
        // A soft inner top highlight, then the white glyph on top.
        const lift = Math.max(0, 1 - (fy - OFF) / (BOX * 0.35)) * 28
        const onGlyph = glyphDist(fx, fy) <= STROKE
        if (onGlyph) { rs += 255; gs += 255; bs += 255 } else {
          rs += Math.min(255, r + lift); gs += Math.min(255, g + lift); bs += Math.min(255, b + lift)
        }
        as += 255
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

const out = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', 'build', 'icon.png')
fs.mkdirSync(path.dirname(out), { recursive: true })
fs.writeFileSync(out, png)
console.log(`wrote ${out} (${SIZE}x${SIZE}, ${(png.length / 1024).toFixed(1)} kB)`)
