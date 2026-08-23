import fs from 'node:fs'
import FSChunkStore from 'fs-chunk-store'

export const PART_SUFFIX = '.part'

/**
 * A chunk store that writes in-progress data to `<name>.part` and renames to
 * the real name only once the torrent completes.
 *
 * WebTorrent otherwise opens the final filename and writes into it from the
 * first byte, so a half-downloaded `movie.mp4` is indistinguishable from a
 * finished one — media players, sync clients and library scanners all happily
 * pick it up.
 *
 * Which name a file uses is decided per file, at open time, by what is already
 * on disk:
 *
 *   final exists   -> use it            (torrent completed, or re-added later)
 *   .part exists   -> use it            (resume it, even if the setting is off)
 *   neither        -> setting decides
 *
 * That keeps the store correct across restarts and re-checks without needing
 * any state of its own. Resuming an existing .part regardless of the setting
 * is what stops a mid-download toggle from orphaning data and re-fetching it,
 * so the store is installed either way and only new files follow the setting.
 */
export function makePartStore (holder, { usePart = true } = {}) {
  return class PartChunkStore {
    constructor (chunkLength, opts = {}) {
      this.chunkLength = chunkLength
      this._opts = opts

      // Let fs-chunk-store resolve the absolute paths rather than duplicating
      // its join/sanitise rules. Constructing it touches no disk: files are
      // opened lazily, so this probe is free and is then discarded.
      const probe = new FSChunkStore(chunkLength, {
        ...opts,
        files: (opts.files || []).map(f => ({ path: f.path, length: f.length, offset: f.offset }))
      })

      this._plan = probe.files.map(f => {
        const partAbs = f.path + PART_SUFFIX
        return {
          partAbs,
          finalAbs: f.path,
          length: f.length,
          offset: f.offset,
          part: exists(f.path) ? false : (exists(partAbs) ? true : usePart)
        }
      })

      this._inner = this._open()
      this.committed = this._plan.every(p => !p.part)
      if (holder) holder.store = this
    }

    _open () {
      // Paths are already absolute here, so opts.path must not be applied again.
      return new FSChunkStore(this.chunkLength, {
        ...this._opts,
        path: null,
        addUID: false,
        files: this._plan.map(p => ({
          path: p.part ? p.partAbs : p.finalAbs,
          length: p.length,
          offset: p.offset
        }))
      })
    }

    put (...args) { return this._inner.put(...args) }
    get (...args) { return this._inner.get(...args) }
    close (cb) { return this._inner.close(cb) }
    destroy (cb) { return this._inner.destroy(cb) }

    /** True while any file is still parked under a .part name. */
    get pending () { return this._plan.filter(p => p.part) }

    /**
     * Close the store, rename every .part to its real name, then reopen on the
     * final paths so seeding continues through the same wrapper. Callers above
     * hold a reference to this object, not to the inner store, so swapping it
     * out underneath them is safe.
     */
    commit (cb) {
      const pending = this.pending
      if (!pending.length) { this.committed = true; return cb(null, []) }

      this._inner.close(() => {
        const renamed = []
        const step = i => {
          if (i >= pending.length) {
            for (const p of pending) p.part = false
            this._inner = this._open()
            this.committed = true
            return cb(null, renamed)
          }
          const p = pending[i]
          fs.rename(p.partAbs, p.finalAbs, err => {
            if (err && err.code === 'ENOENT') {
              // Zero-length or never-written file; nothing to move.
              p.part = false
              return step(i + 1)
            }
            if (err) {
              // Reopen on whatever is actually there so the torrent keeps working.
              this._inner = this._open()
              return cb(err, renamed)
            }
            renamed.push(p.finalAbs)
            step(i + 1)
          })
        }
        step(0)
      })
    }
  }
}

function exists (p) {
  try { return fs.existsSync(p) } catch { return false }
}
