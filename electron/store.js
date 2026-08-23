import fs from 'node:fs'
import path from 'node:path'
import os from 'node:os'

/**
 * Flat JSON persistence for preferences and the resume session.
 * Writes are debounced and atomic (write temp + rename) so a crash mid-save
 * can never leave a truncated settings file behind.
 */

export const DEFAULT_SETTINGS = {
  downloadPath: path.join(os.homedir(), 'Downloads'),
  askWhereToSave: true,
  startTorrentsAutomatically: true,
  sequentialDownload: false,
  partFiles: true,        // write to <name>.part until the torrent completes

  // Bandwidth (kB/s; 0 = unlimited)
  maxDownloadRate: 0,
  maxUploadRate: 0,
  globalMaxConnections: 200,
  maxUploadSlots: 4,

  // Connection
  listenPort: 0,          // 0 = random
  randomizePort: true,
  enableDHT: true,
  enablePEX: true,
  enableLSD: true,
  enableUPnP: true,
  enableUTP: true,

  // Queueing
  maxActiveTorrents: 8,
  maxActiveDownloads: 5,
  seedRatioLimit: 0,      // 0 = seed forever
  seedTimeLimit: 0,       // minutes, 0 = forever

  // UI
  theme: 'classic',       // 'classic' | 'graphite'
  confirmOnDelete: true,
  notifyOnComplete: true,
  showSpeedInDock: true,
  altSpeedEnabled: false,
  altDownloadRate: 100,
  altUploadRate: 20
}

export class Store {
  constructor (dir) {
    this.dir = dir
    this.file = path.join(dir, 'ztorrent-state.json')
    this.tmp = this.file + '.tmp'
    this._timer = null
    this.legacyFile = Store._findLegacy(dir, this.file)
    this.data = this._read()
  }

  /**
   * The app used to be called Murmur, which means Electron handed it a
   * different userData directory and a different state filename. If we have no
   * state of our own yet, adopt the old library rather than starting empty.
   * The old file is left untouched, so downgrading stays possible.
   */
  static _findLegacy (dir, current) {
    if (fs.existsSync(current)) return null
    const candidates = [
      path.join(dir, 'murmur-state.json'),
      path.join(path.dirname(dir), 'Murmur', 'murmur-state.json')
    ]
    return candidates.find(f => fs.existsSync(f)) || null
  }

  _read () {
    let parsed = {}
    try {
      parsed = JSON.parse(fs.readFileSync(this.legacyFile || this.file, 'utf8'))
    } catch {
      parsed = {}
    }
    return {
      settings: { ...DEFAULT_SETTINGS, ...(parsed.settings || {}) },
      torrents: Array.isArray(parsed.torrents) ? parsed.torrents : [],
      labels: Array.isArray(parsed.labels) ? parsed.labels : [],
      columns: parsed.columns || null,
      window: parsed.window || null
    }
  }

  get settings () { return this.data.settings }

  patchSettings (patch) {
    Object.assign(this.data.settings, patch)
    this.save()
    return this.data.settings
  }

  set (key, value) {
    this.data[key] = value
    this.save()
  }

  /** Debounced atomic write. */
  save () {
    if (this._timer) return
    this._timer = setTimeout(() => {
      this._timer = null
      this.flush()
    }, 400)
  }

  flush () {
    if (this._timer) { clearTimeout(this._timer); this._timer = null }
    try {
      fs.mkdirSync(this.dir, { recursive: true })
      fs.writeFileSync(this.tmp, JSON.stringify(this.data, null, 2))
      fs.renameSync(this.tmp, this.file)
    } catch (err) {
      console.error('[store] save failed:', err.message)
    }
  }
}
