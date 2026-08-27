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
  // Where the last torrent was actually saved. The Add dialog offers this
  // instead of downloadPath, so a run of torrents going to the same drive is
  // picked once rather than once each. Cleared when downloadPath is changed.
  lastSavePath: '',
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
  // Off by default: LSD multicasts each torrent's infohash in the clear to
  // every device on the LAN, and only ever finds peers on that same LAN.
  enableLSD: false,
  enableUPnP: true,
  enableUTP: true,
  encryption: 1,          // 0 = off, 1 = prefer MSE (plaintext fallback), 2 = require MSE

  // Proxy. SOCKS5 only, and it takes DHT, LSD, uTP, UPnP and udp:// trackers
  // down with it -- none of them can be routed, so they are not sent direct.
  proxyEnabled: false,
  proxyHost: '',
  proxyPort: 1080,
  proxyUsername: '',
  proxyPassword: '',
  bindInterface: '',      // '' = any; otherwise an interface name, e.g. 'utun4'

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

/**
 * Settings that must not be written to disk in the clear. The value lives in
 * memory under its own name and is persisted, encrypted, under `<key>Enc`.
 */
const SECRET_KEYS = ['proxyPassword']

export class Store {
  /**
   * `secrets` is an optional { encrypt, decrypt } pair -- Electron's safeStorage
   * in the app, absent in tests and tools. Without it the secret keys fall back
   * to plaintext, which is what they were before, rather than being lost.
   */
  constructor (dir, secrets = null) {
    this.secrets = secrets
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
    const settings = { ...DEFAULT_SETTINGS, ...(parsed.settings || {}) }
    for (const key of SECRET_KEYS) {
      const sealed = settings[`${key}Enc`]
      if (!sealed) continue
      let opened = null
      if (this.secrets) {
        try { opened = this.secrets.decrypt(sealed) } catch { opened = null }
      }
      if (opened === null) {
        // Keychain denied, or this build has no codec. Hold the sealed value
        // as-is so the next save writes it back rather than wiping it.
        settings[key] = ''
      } else {
        settings[key] = opened
        delete settings[`${key}Enc`]
      }
    }
    return {
      settings,
      torrents: Array.isArray(parsed.torrents) ? parsed.torrents : [],
      labels: Array.isArray(parsed.labels) ? parsed.labels : [],
      columns: parsed.columns || null,
      window: parsed.window || null
    }
  }

  get settings () { return this.data.settings }

  patchSettings (patch) {
    // An explicit new value for a secret replaces whatever was sealed before,
    // including when it is cleared to ''.
    for (const key of SECRET_KEYS) {
      if (key in patch) delete this.data.settings[`${key}Enc`]
    }
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

  /** The on-disk shape: secrets sealed, never written under their own name. */
  _serialise () {
    const settings = { ...this.data.settings }
    let changed = false
    for (const key of SECRET_KEYS) {
      const value = settings[key]
      if (!this.secrets) continue          // no codec: leave it as it was
      delete settings[key]
      changed = true
      if (!value) continue                 // empty, or a sealed value we could not open
      try { settings[`${key}Enc`] = this.secrets.encrypt(value) } catch { /* drop it */ }
    }
    return changed ? { ...this.data, settings } : this.data
  }

  flush () {
    if (this._timer) { clearTimeout(this._timer); this._timer = null }
    try {
      fs.mkdirSync(this.dir, { recursive: true })
      fs.writeFileSync(this.tmp, JSON.stringify(this._serialise(), null, 2))
      fs.renameSync(this.tmp, this.file)
    } catch (err) {
      console.error('[store] save failed:', err.message)
    }
  }
}
