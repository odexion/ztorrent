/** Formatting helpers. Units and rounding follow what µTorrent prints. */

const SIZE_UNITS = ['B', 'kB', 'MB', 'GB', 'TB', 'PB']

export function bytes (n, blankZero = false) {
  if (!isFinite(n) || n < 0) return ''
  if (n === 0) return blankZero ? '' : '0 B'
  let i = 0
  let v = n
  while (v >= 1024 && i < SIZE_UNITS.length - 1) { v /= 1024; i++ }
  const dp = i === 0 ? 0 : (v >= 100 ? 1 : 2)
  return `${v.toFixed(dp)} ${SIZE_UNITS[i]}`
}

export function speed (n, blankZero = true) {
  if (!n || n < 1) return blankZero ? '' : '0 B/s'
  return `${bytes(n)}/s`
}

export function eta (seconds) {
  if (!isFinite(seconds) || seconds <= 0) return '∞'
  const s = Math.round(seconds / 1000)
  if (s < 1) return '<1s'
  if (s > 60 * 60 * 24 * 365) return '∞'
  const d = Math.floor(s / 86400)
  const h = Math.floor((s % 86400) / 3600)
  const m = Math.floor((s % 3600) / 60)
  const sec = s % 60
  if (d) return `${d}d ${h}h`
  if (h) return `${h}h ${m}m`
  if (m) return `${m}m ${sec}s`
  return `${sec}s`
}

export function pct (v, dp = 1) {
  return `${(Math.max(0, Math.min(1, v)) * 100).toFixed(dp)}%`
}

export function ratio (v) {
  if (!isFinite(v)) return '∞'
  return v.toFixed(3)
}

const pad = n => String(n).padStart(2, '0')

export function datetime (ms) {
  if (!ms) return ''
  const d = new Date(ms)
  return `${pad(d.getDate())}/${pad(d.getMonth() + 1)}/${d.getFullYear()} ${pad(d.getHours())}:${pad(d.getMinutes())}`
}

export function clock (ms) {
  const d = new Date(ms)
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`
}

export function duration (ms) {
  if (!ms) return ''
  const s = Math.floor(ms / 1000)
  const d = Math.floor(s / 86400)
  const h = Math.floor((s % 86400) / 3600)
  const m = Math.floor((s % 3600) / 60)
  if (d) return `${d}d ${h}h`
  if (h) return `${h}h ${m}m`
  return `${m}m`
}

export function esc (s) {
  return String(s ?? '')
    .replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;').replace(/'/g, '&#39;')
}

/** Splits an "hh:mm" style piece-size selector value into a human label. */
export function pieceLabel (bytesPerPiece) {
  if (!bytesPerPiece) return 'auto'
  return bytes(bytesPerPiece)
}

export function debounce (fn, wait) {
  let t
  return (...args) => { clearTimeout(t); t = setTimeout(() => fn(...args), wait) }
}

/** Natural sort that keeps "File 2" before "File 10". */
const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: 'base' })
export const compareText = (a, b) => collator.compare(a ?? '', b ?? '')
