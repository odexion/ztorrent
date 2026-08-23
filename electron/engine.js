import { EventEmitter } from 'node:events'
import { makePartStore, PART_SUFFIX } from './part-store.js'
import { egressPolicy, useEgress, guardedOptions, samePolicy, closeInbound } from './egress.js'
import fs from 'node:fs'
import path from 'node:path'
import crypto from 'node:crypto'
import WebTorrent from 'webtorrent'
import parseTorrent from 'parse-torrent'

/** Torrent lifecycle states, mirroring the vocabulary uTorrent shows in its Status column. */
export const State = {
  DOWNLOADING: 'downloading',
  SEEDING: 'seeding',
  PAUSED: 'paused',
  STOPPED: 'stopped',
  QUEUED: 'queued',
  CHECKING: 'checking',
  METADATA: 'metadata',
  FINISHED: 'finished',
  ERROR: 'error'
}

const KB = 1024

/**
 * Wraps a WebTorrent client with the bookkeeping a desktop client needs:
 * stable ids that survive restarts, stopped-but-remembered torrents, a
 * download queue, labels, and a serialisable snapshot for the renderer.
 *
 * A "record" is our persistent view of a torrent. `record.torrent` is the live
 * WebTorrent instance and is null whenever the torrent is stopped.
 */
export class Engine extends EventEmitter {
  constructor (store) {
    super()
    this.store = store
    this.records = new Map()   // id -> record
    this.client = null
    this.logLines = []
    this._orderSeq = 0
  }

  // ---------------------------------------------------------------- lifecycle

  start () {
    const s = this.store.settings
    // Install before the client exists: the egress policy decides what the
    // client is even allowed to switch on.
    this.egress = egressPolicy(s)
    const egressLabel = useEgress(this.egress)

    this.client = new WebTorrent({
      maxConns: s.globalMaxConnections,
      dht: s.enableDHT,
      lsd: s.enableLSD,
      utPex: s.enablePEX,
      utp: s.enableUTP,
      // Message Stream Encryption. WebTorrent defaults this to 1; naming it
      // here keeps the choice visible and lets the user require it.
      secure: s.encryption ?? 1,
      natUpnp: s.enableUPnP,
      natPmp: s.enableUPnP,
      torrentPort: s.randomizePort ? 0 : s.listenPort,
      downloadLimit: s.maxDownloadRate > 0 ? s.maxDownloadRate * KB : -1,
      uploadLimit: s.maxUploadRate > 0 ? s.maxUploadRate * KB : -1,
      ...guardedOptions(this.egress)
    })

    this.client.on('error', err => this.log(`Client error: ${err.message || err}`, 'error'))
    this.log('ztorrent started. Listening for peers.')
    if ((s.encryption ?? 1) === 2) {
      this.log('Requiring protocol encryption: peers that will not encrypt are refused. ' +
               'This shrinks the pool of usable peers.')
    }
    if (egressLabel) {
      this.log(`Trackers, web seeds and peer connections go out via ${egressLabel}.`)
      const off = ['local discovery', 'uTP', 'port mapping', 'udp:// trackers']
      if (this.egress.proxy) off.unshift('DHT')
      this.log(`${off.join(', ')} are off while this is on -- none of them can be routed.`)
      if (this.egress.proxy) {
        // Only once the pool has finished binding: the client's 'listening'
        // step is what releases every torrent into discovery, and closing the
        // server before it runs would stall all of them.
        this.client.once('listening', () => {
          if (closeInbound(this.client)) {
            this.log('Inbound listeners closed: with a proxy, connections are outgoing only.')
          }
        })
      }
    }

    if (this.client.dht) {
      this.client.dht.on('ready', () => this.log('DHT bootstrap complete.'))
    }

    this.restoreSession()
  }

  async destroy () {
    this.persist()
    this.store.flush()
    await new Promise(resolve => {
      if (!this.client) return resolve()
      this.client.destroy(() => resolve())
    })
  }

  log (message, level = 'info') {
    const line = { time: Date.now(), message, level }
    this.logLines.push(line)
    if (this.logLines.length > 800) this.logLines.splice(0, this.logLines.length - 800)
    this.emit('log', line)
  }

  // ------------------------------------------------------------------ session

  restoreSession () {
    const saved = this.store.data.torrents || []
    for (const entry of saved) {
      const record = {
        id: entry.id || crypto.randomUUID(),
        infoHash: entry.infoHash || null,
        name: entry.name || 'Unknown',
        magnetURI: entry.magnetURI || null,
        torrentFile: entry.torrentFile || null,   // base64 .torrent
        savePath: entry.savePath,
        label: entry.label || '',
        order: entry.order ?? this._orderSeq++,
        addedOn: entry.addedOn || Date.now(),
        completedOn: entry.completedOn || 0,
        uploadedBase: entry.uploadedBase || 0,
        downloadedBase: entry.downloadedBase || 0,
        length: entry.length || 0,
        wanted: entry.wanted || null,             // array of wanted file indices
        priorities: entry.priorities || {},       // fileIndex -> 0|1|2 (skip/normal/high)
        sequential: !!entry.sequential,
        state: entry.state === State.STOPPED ? State.STOPPED : entry.state,
        wantStart: entry.state !== State.STOPPED && entry.state !== State.PAUSED,
        progress: entry.progress || 0,
        error: null,
        torrent: null
      }
      this._orderSeq = Math.max(this._orderSeq, record.order + 1)
      this.records.set(record.id, record)
      if (record.wantStart) this._spawn(record)
      else record.state = entry.state === State.PAUSED ? State.PAUSED : State.STOPPED
    }
    if (saved.length) this.log(`Restored ${saved.length} torrent(s) from the previous session.`)
    this.pumpQueue()
  }

  persist () {
    const rows = [...this.records.values()].map(r => ({
      id: r.id,
      infoHash: r.infoHash,
      name: r.name,
      magnetURI: r.magnetURI,
      torrentFile: r.torrentFile,
      savePath: r.savePath,
      label: r.label,
      order: r.order,
      addedOn: r.addedOn,
      completedOn: r.completedOn,
      uploadedBase: r.uploadedBase + (r.torrent ? r.torrent.uploaded : 0),
      downloadedBase: r.torrent ? r.torrent.downloaded : r.downloadedBase,
      length: r.torrent ? r.torrent.length : r.length,
      wanted: r.wanted,
      priorities: r.priorities,
      sequential: r.sequential,
      progress: r.torrent ? r.torrent.progress : r.progress,
      state: r.state
    }))
    this.store.set('torrents', rows)
  }

  // ---------------------------------------------------------------- adding

  /**
   * `source` may be a magnet URI, an http(s) link to a .torrent, an absolute
   * path to a .torrent on disk, or a Buffer holding raw .torrent bytes.
   */
  async add (source, opts = {}) {
    const settings = this.store.settings
    let torrentFile = null
    let magnetURI = null
    let parsed = null

    try {
      // create-torrent hands back a plain Uint8Array, and drag-and-drop can
      // deliver an ArrayBuffer, so normalise any binary form to a Buffer.
      if (source instanceof Uint8Array || source instanceof ArrayBuffer) {
        const buf = Buffer.from(
          source instanceof ArrayBuffer ? new Uint8Array(source) : source)
        torrentFile = buf.toString('base64')
        parsed = await parseTorrent(buf)
      } else if (typeof source === 'string' && source.startsWith('magnet:')) {
        magnetURI = source
        parsed = await parseTorrent(source)
      } else if (typeof source === 'string' && /^https?:\/\//.test(source)) {
        const res = await fetch(source)
        if (!res.ok) throw new Error(`HTTP ${res.status}`)
        const buf = Buffer.from(await res.arrayBuffer())
        torrentFile = buf.toString('base64')
        parsed = await parseTorrent(buf)
      } else if (typeof source === 'string') {
        const buf = fs.readFileSync(source)
        torrentFile = buf.toString('base64')
        parsed = await parseTorrent(buf)
      }
    } catch (err) {
      this.log(`Could not read torrent: ${err.message}`, 'error')
      throw err
    }

    if (!parsed) {
      const err = new Error('Unrecognised torrent source')
      this.log(`Could not add torrent: ${err.message}`, 'error')
      throw err
    }

    const infoHash = parsed.infoHash || null
    const existing = infoHash && [...this.records.values()].find(r => r.infoHash === infoHash)
    if (existing) {
      this.log(`"${existing.name}" is already in the list.`, 'warn')
      return { duplicate: true, id: existing.id }
    }

    const record = {
      id: crypto.randomUUID(),
      infoHash,
      name: parsed.name || 'Downloading metadata',
      magnetURI: magnetURI || (parsed.infoHash ? `magnet:?xt=urn:btih:${parsed.infoHash}` : null),
      torrentFile,
      savePath: opts.savePath || settings.downloadPath,
      label: opts.label || '',
      order: this._orderSeq++,
      addedOn: Date.now(),
      completedOn: 0,
      uploadedBase: 0,
      downloadedBase: 0,
      length: parsed.length || 0,
      wanted: opts.wanted || null,
      priorities: opts.priorities || {},
      sequential: opts.sequential ?? settings.sequentialDownload,
      state: State.QUEUED,
      wantStart: opts.paused ? false : settings.startTorrentsAutomatically,
      progress: 0,
      error: null,
      torrent: null
    }

    this.records.set(record.id, record)
    this.log(`Added "${record.name}".`)

    if (record.wantStart) this.pumpQueue()
    else record.state = State.PAUSED

    this.persist()
    this.emit('changed')
    return { id: record.id }
  }

  /** Boots the live WebTorrent instance for a record. */
  _spawn (record) {
    if (record.torrent) return
    const source = record.torrentFile
      ? Buffer.from(record.torrentFile, 'base64')
      : record.magnetURI

    if (!source) {
      record.state = State.ERROR
      record.error = 'No torrent data'
      return
    }

    const addOpts = {
      path: record.savePath,
      strategy: record.sequential ? 'sequential' : 'rarest',
      uploads: this.store.settings.maxUploadSlots
    }
    // The store hands itself back through this holder so completion can ask it
    // to rename the .part files.
    const partHolder = {}
    addOpts.store = makePartStore(partHolder, {
      usePart: this.store.settings.partFiles !== false
    })
    record.partHolder = partHolder
    if (Array.isArray(record.wanted) && record.wanted.length) addOpts.so = record.wanted

    record.state = State.CHECKING
    record.error = null

    let torrent
    try {
      torrent = this.client.add(source, addOpts)
    } catch (err) {
      record.state = State.ERROR
      record.error = err.message
      this.log(`Failed to start "${record.name}": ${err.message}`, 'error')
      return
    }

    record.torrent = torrent
    this._wire(record, torrent)
  }

  _wire (record, torrent) {
    torrent.on('infoHash', () => {
      record.infoHash = torrent.infoHash
    })

    torrent.on('metadata', () => {
      record.name = torrent.name
      record.length = torrent.length
      try {
        record.torrentFile = Buffer.from(torrent.torrentFile).toString('base64')
      } catch { /* metadata not serialisable yet */ }
      this._applyPriorities(record)
      this.log(`Received metadata for "${record.name}".`)
      this.persist()
      this.emit('changed')
    })

    torrent.on('ready', () => {
      record.name = torrent.name
      record.length = torrent.length
      this._applyPriorities(record)
      record.state = torrent.done
        ? State.SEEDING
        : (torrent.paused ? State.PAUSED : State.DOWNLOADING)
      this.emit('changed')
    })

    torrent.on('done', () => {
      if (!record.completedOn) record.completedOn = Date.now()
      record.state = State.SEEDING
      this._commitPartFiles(record, () => {
        this.log(`"${record.name}" finished downloading.`)
        this.persist()
        this.emit('complete', { id: record.id, name: record.name, path: record.savePath })
        this.pumpQueue()
      })
    })

    torrent.on('error', err => {
      record.state = State.ERROR
      record.error = err.message || String(err)
      this.log(`Error on "${record.name}": ${record.error}`, 'error')
      this.emit('changed')
    })

    // Trackers report swarm size in their announce response; stash it per URL
    // so the Trackers tab can show real seed/leech counts.
    record.trackerStats = record.trackerStats || {}
    const hookTracker = () => {
      const tc = torrent.discovery?.tracker
      if (!tc || tc._ztorrentHooked) return
      tc._ztorrentHooked = true
      tc.on('update', data => {
        if (!data?.announce) return
        record.trackerStats[data.announce] = {
          complete: data.complete ?? -1,
          incomplete: data.incomplete ?? -1,
          at: Date.now()
        }
      })
    }
    hookTracker()
    torrent.on('ready', hookTracker)

    torrent.on('warning', err => {
      const msg = err.message || String(err)
      // Tracker chatter is noisy; keep it out of the user-facing log.
      if (/tracker|announce|ECONN|ETIMEDOUT|EHOSTUNREACH|socket/i.test(msg)) return
      this.log(msg, 'warn')
    })
  }

  /** Re-applies the record's per-file skip/normal/high choices to the live torrent. */
  _applyPriorities (record) {
    const torrent = record.torrent
    if (!torrent || !torrent.files || !torrent.files.length) return
    const prios = record.priorities || {}
    const hasChoices = Object.keys(prios).length > 0
    if (!hasChoices) return

    torrent.files.forEach((file, i) => {
      const p = prios[i] ?? 1
      if (p === 0) file.deselect()
      else file.select(p === 2 ? 1 : 0)
    })
    record.wanted = torrent.files
      .map((_, i) => i)
      .filter(i => (prios[i] ?? 1) !== 0)
  }

  // --------------------------------------------------------------- queueing

  /** Starts as many wanting-to-run torrents as the queue settings allow. */
  pumpQueue () {
    const s = this.store.settings
    const all = [...this.records.values()].sort((a, b) => a.order - b.order)

    let activeDownloads = 0
    let activeTotal = 0
    for (const r of all) {
      if (!r.torrent || r.state === State.PAUSED) continue
      activeTotal++
      if (r.state !== State.SEEDING && r.state !== State.FINISHED) activeDownloads++
    }

    for (const r of all) {
      if (r.torrent || !r.wantStart) continue
      const willSeed = r.progress >= 1
      if (activeTotal >= s.maxActiveTorrents) { r.state = State.QUEUED; continue }
      if (!willSeed && activeDownloads >= s.maxActiveDownloads) { r.state = State.QUEUED; continue }
      this._spawn(r)
      activeTotal++
      if (!willSeed) activeDownloads++
    }
    this.emit('changed')
  }

  // ------------------------------------------------------------- commands

  startTorrent (id) {
    const r = this.records.get(id)
    if (!r) return
    r.wantStart = true
    if (r.torrent) {
      r.torrent.resume()
      this._reselect(r)
      r.state = r.torrent.done ? State.SEEDING : State.DOWNLOADING
    } else {
      r.state = State.QUEUED
      this.pumpQueue()
    }
    this.log(`Started "${r.name}".`)
    this.persist()
    this.emit('changed')
  }

  pauseTorrent (id) {
    const r = this.records.get(id)
    if (!r || !r.torrent) return
    r.torrent.pause()
    this._haltTransfer(r.torrent)
    r.state = State.PAUSED
    r.wantStart = false
    this.log(`Paused "${r.name}".`)
    this.persist()
    this.emit('changed')
  }

  /**
   * Stops a paused torrent asking for any more data.
   *
   * torrent.pause() by itself only refuses NEW peers — wires we are already
   * connected to keep feeding us at full speed. The public deselect() is no
   * help either: it removes only an exactly-matching range, and WebTorrent
   * selects the whole torrent as a single entry, so per-file deselects are
   * silent no-ops. Clearing the selection set and recomputing interest is what
   * actually stops the requests. Blocks already in flight (a few MB across a
   * full swarm) still land, then the transfer drops to zero within seconds.
   */
  _haltTransfer (torrent) {
    if (!torrent) return
    try {
      torrent._selections?.clear()
      torrent._updateInterest?.()
    } catch (err) {
      this.log(`Could not fully pause transfer: ${err.message}`, 'warn')
    }
    if (torrent.pieces) torrent.deselect(0, torrent.pieces.length - 1)
  }

  /** Restores what a torrent should be asking for after a pause. */
  _reselect (record) {
    const torrent = record.torrent
    if (!torrent || !torrent.files || !torrent.files.length) return
    if (Object.keys(record.priorities || {}).length) this._applyPriorities(record)
    else torrent.files.forEach(f => f.select())
  }

  /** Stop tears the swarm down entirely but keeps the torrent in the list. */
  stopTorrent (id) {
    const r = this.records.get(id)
    if (!r) return
    r.wantStart = false
    if (r.torrent) {
      r.progress = r.torrent.progress
      r.uploadedBase += r.torrent.uploaded
      r.downloadedBase = r.torrent.downloaded
      r.length = r.torrent.length || r.length
      this.client.remove(r.torrent, { destroyStore: false }, () => {})
      r.torrent = null
    }
    r.state = State.STOPPED
    this.log(`Stopped "${r.name}".`)
    this.persist()
    this.emit('changed')
    this.pumpQueue()
  }

  removeTorrent (id, deleteData = false) {
    const r = this.records.get(id)
    if (!r) return
    const savePath = r.savePath
    const name = r.name
    if (r.torrent) {
      this.client.remove(r.torrent, { destroyStore: deleteData }, () => {})
      r.torrent = null
    } else if (deleteData && savePath && name && name !== 'Downloading metadata') {
      const target = path.join(savePath, name)
      try {
        if (fs.existsSync(target)) fs.rmSync(target, { recursive: true, force: true })
      } catch (err) {
        this.log(`Could not delete data for "${name}": ${err.message}`, 'error')
      }
    }
    this.records.delete(id)
    this.log(deleteData ? `Removed "${name}" and deleted its data.` : `Removed "${name}".`)
    this.persist()
    this.emit('changed')
    this.pumpQueue()
  }

  /**
   * Drop the .part suffix now that every piece is present. Seeding continues
   * through the same store object, which reopens on the final paths.
   */
  _commitPartFiles (record, done) {
    const store = record.partHolder?.store
    if (!store || store.committed) return done()
    store.commit((err, renamed) => {
      if (err) {
        this.log(`Could not rename ${PART_SUFFIX} files for "${record.name}": ${err.message}`, 'error')
      } else if (renamed.length) {
        this.log(`Finalised ${renamed.length} file(s) for "${record.name}".`)
      }
      this.emit('changed')
      done()
    })
  }

  /** Force re-check: re-hash every piece against what is on disk. */
  async recheck (id) {
    const r = this.records.get(id)
    if (!r) return
    if (r.torrent) {
      r.state = State.CHECKING
      this.emit('changed')
      this.log(`Re-checking "${r.name}"...`)
      try {
        await r.torrent.rescanFiles()
        r.state = r.torrent.done ? State.SEEDING : (r.torrent.paused ? State.PAUSED : State.DOWNLOADING)
        this.log(`Re-check of "${r.name}" complete (${(r.torrent.progress * 100).toFixed(1)}%).`)
      } catch (err) {
        this.log(`Re-check failed: ${err.message}`, 'error')
      }
      this.emit('changed')
    } else {
      // Stopped torrents are re-checked by simply restarting them; WebTorrent
      // verifies existing pieces on add.
      this.startTorrent(id)
    }
  }

  setLabel (id, label) {
    const r = this.records.get(id)
    if (!r) return
    r.label = label
    const labels = new Set(this.store.data.labels || [])
    if (label) labels.add(label)
    this.store.set('labels', [...labels])
    this.persist()
    this.emit('changed')
  }

  setSequential (id, on) {
    const r = this.records.get(id)
    if (!r) return
    r.sequential = !!on
    if (r.torrent) r.torrent.strategy = on ? 'sequential' : 'rarest'
    this.persist()
    this.emit('changed')
  }

  setFilePriority (id, fileIndex, priority) {
    const r = this.records.get(id)
    if (!r) return
    r.priorities[fileIndex] = priority
    if (r.torrent && r.torrent.files[fileIndex]) {
      const file = r.torrent.files[fileIndex]
      if (priority === 0) file.deselect()
      else file.select(priority === 2 ? 1 : 0)
      r.wanted = r.torrent.files.map((_, i) => i).filter(i => (r.priorities[i] ?? 1) !== 0)
    }
    this.persist()
    this.emit('changed')
  }

  addTracker (id, url) {
    const r = this.records.get(id)
    if (!r || !r.torrent) return false
    try {
      r.torrent.announce.push(url)
      if (r.torrent.discovery?.tracker) r.torrent.discovery.tracker.add?.(url)
      this.log(`Added tracker ${url} to "${r.name}".`)
      return true
    } catch (err) {
      this.log(`Could not add tracker: ${err.message}`, 'error')
      return false
    }
  }

  addPeer (id, addr) {
    const r = this.records.get(id)
    if (!r || !r.torrent) return false
    const ok = r.torrent.addPeer(addr)
    this.log(ok ? `Added peer ${addr}.` : `Peer ${addr} rejected.`, ok ? 'info' : 'warn')
    return ok
  }

  reannounce (id) {
    const r = this.records.get(id)
    if (r?.torrent?.discovery?.tracker) {
      r.torrent.discovery.tracker.update()
      this.log(`Re-announced "${r.name}" to its trackers.`)
    }
  }

  /** Moves a torrent up or down the queue. */
  moveQueue (id, delta) {
    const all = [...this.records.values()].sort((a, b) => a.order - b.order)
    const i = all.findIndex(r => r.id === id)
    const j = i + delta
    if (i < 0 || j < 0 || j >= all.length) return
    const a = all[i]; const b = all[j]
    const tmp = a.order; a.order = b.order; b.order = tmp
    this.persist()
    this.emit('changed')
  }

  /**
   * Pauses seeding torrents that have met the configured share-ratio or
   * seed-time goal. Called once per tick from the main process.
   */
  enforceSeedGoals () {
    const s = this.store.settings
    const ratioGoal = s.seedRatioLimit > 0 ? s.seedRatioLimit / 100 : 0
    const timeGoalMs = s.seedTimeLimit > 0 ? s.seedTimeLimit * 60_000 : 0
    if (!ratioGoal && !timeGoalMs) return

    for (const r of this.records.values()) {
      if (!r.torrent || r.state !== State.SEEDING || r.torrent.paused) continue
      const downloaded = r.torrent.downloaded || r.downloadedBase
      const uploaded = r.uploadedBase + r.torrent.uploaded
      const ratio = shareRatio(downloaded, uploaded, r.torrent.length || r.length)

      const hitRatio = ratioGoal && ratio >= ratioGoal
      const hitTime = timeGoalMs && r.completedOn && Date.now() - r.completedOn >= timeGoalMs
      if (!hitRatio && !hitTime) continue

      this.log(hitRatio
        ? `"${r.name}" reached its share ratio goal (${ratio.toFixed(2)}); seeding stopped.`
        : `"${r.name}" reached its seeding time goal; seeding stopped.`)
      this.pauseTorrent(r.id)
    }
  }

  /** True when the new settings need a restart to take effect. */
  egressChanged (settings) {
    return !samePolicy(this.egress, egressPolicy(settings))
  }

  applySettings (settings) {
    if (!this.client) return
    this.client.throttleDownload(settings.maxDownloadRate > 0 ? settings.maxDownloadRate * KB : -1)
    this.client.throttleUpload(settings.maxUploadRate > 0 ? settings.maxUploadRate * KB : -1)
    this.client.maxConns = settings.globalMaxConnections
    // Read afresh for every outgoing peer, so this one needs no restart.
    this.client.secure = settings.encryption ?? 1
    for (const r of this.records.values()) {
      if (r.torrent) r.torrent._rechokeNumSlots = settings.maxUploadSlots
    }
    this.pumpQueue()
  }

  setAltSpeed (on) {
    const s = this.store.settings
    if (on) {
      this.client.throttleDownload(s.altDownloadRate * KB)
      this.client.throttleUpload(s.altUploadRate * KB)
      this.log('Alternate speed limits enabled.')
    } else {
      this.client.throttleDownload(s.maxDownloadRate > 0 ? s.maxDownloadRate * KB : -1)
      this.client.throttleUpload(s.maxUploadRate > 0 ? s.maxUploadRate * KB : -1)
      this.log('Alternate speed limits disabled.')
    }
  }

  // ------------------------------------------------------------- snapshots

  /** Compact per-torrent state for the main list. Called ~1x/second. */
  snapshot () {
    const rows = []
    for (const r of this.records.values()) {
      const t = r.torrent
      const done = t ? t.progress >= 1 : r.progress >= 1
      const uploaded = r.uploadedBase + (t ? t.uploaded : 0)
      const downloaded = t ? t.downloaded : r.downloadedBase
      const length = t?.length || r.length
      const wantedLength = t
        ? t.files.reduce((sum, f, i) => sum + ((r.priorities[i] ?? 1) === 0 ? 0 : f.length), 0)
        : length

      let state = r.state
      if (t) {
        if (r.state !== State.CHECKING) {
          if (!t.ready && !t.metadata) state = State.METADATA
          else if (t.paused) state = State.PAUSED
          else state = done ? State.SEEDING : State.DOWNLOADING
        }
      }

      rows.push({
        id: r.id,
        infoHash: r.infoHash || '',
        name: r.name,
        order: r.order,
        state,
        error: r.error,
        label: r.label,
        savePath: r.savePath,
        size: length,
        wantedSize: wantedLength,
        done: t ? t.progress : r.progress,
        downloaded,
        uploaded,
        downloadSpeed: t ? t.downloadSpeed : 0,
        uploadSpeed: t ? t.uploadSpeed : 0,
        numPeers: t ? t.numPeers : 0,
        seeds: t ? t.wires.filter(w => w.isSeeder).length : 0,
        peers: t ? t.wires.filter(w => !w.isSeeder).length : 0,
        seedsTotal: t?.discovery?.tracker ? (t._seedsTotal ?? 0) : 0,
        eta: t ? t.timeRemaining : Infinity,
        ratio: shareRatio(downloaded, uploaded, length),
        availability: t ? computeAvailability(t) : 0,
        addedOn: r.addedOn,
        completedOn: r.completedOn,
        sequential: r.sequential,
        magnetURI: t?.magnetURI || r.magnetURI
      })
    }
    rows.sort((a, b) => a.order - b.order)
    return rows
  }

  /** Rich detail for the currently selected torrent only. */
  details (id) {
    const r = this.records.get(id)
    if (!r) return null
    const t = r.torrent
    const base = {
      id: r.id,
      name: r.name,
      infoHash: r.infoHash || '',
      savePath: r.savePath,
      comment: t?.comment || '',
      createdBy: t?.createdBy || '',
      createdOn: t?.created ? new Date(t.created).getTime() : 0,
      pieceLength: t?.pieceLength || 0,
      pieceCount: t?.pieces?.length || 0,
      private: !!t?.private,
      sequential: r.sequential,
      trackers: [],
      peers: [],
      files: [],
      pieces: null,
      have: 0
    }
    if (!t) {
      base.files = (r.wanted || []).map(i => ({ index: i }))
      return base
    }

    // Trackers
    const announce = t.announce || []
    const trackerClient = t.discovery?.tracker
    for (const url of announce) {
      const normalised = url.endsWith('/') ? url.slice(0, -1) : url
      const tr = trackerClient?._trackers?.find(x => x.announceUrl === normalised)
      const stats = r.trackerStats?.[normalised] || r.trackerStats?.[url]
      base.trackers.push({
        url,
        status: !tr ? 'Not contacted yet' : (tr.destroyed ? 'Not working' : (stats ? 'Working' : 'Announcing…')),
        seeds: stats?.complete ?? -1,
        peers: stats?.incomplete ?? -1,
        interval: tr?.interval ? Math.round(tr.interval / 1000) : 0
      })
    }
    if (t.urlList?.length) {
      for (const url of t.urlList) {
        base.trackers.push({ url, status: 'Web Seed', seeds: -1, peers: -1, interval: 0 })
      }
    }
    if (this.client.dht) {
      base.trackers.unshift({
        url: '[DHT]',
        status: this.client.dht.ready ? 'Working' : 'Bootstrapping',
        seeds: -1, peers: this.client.dht.nodes?.count?.() ?? -1, interval: 0
      })
    }
    base.trackers.unshift({ url: '[Peer Exchange]', status: 'Working', seeds: -1, peers: -1, interval: 0 })
    base.trackers.unshift({ url: '[Local Peer Discovery]', status: 'Working', seeds: -1, peers: -1, interval: 0 })

    // Peers
    for (const wire of t.wires) {
      base.peers.push({
        address: wire.type === 'webSeed'
          ? (wire.remoteAddress || 'web seed')
          : (wire.remoteAddress || '?'),
        port: wire.remotePort || 0,
        client: peerClient(wire),
        flags: peerFlags(wire),
        progress: wire.peerPieces ? bitfieldRatio(wire.peerPieces, t.pieces.length) : 0,
        downSpeed: wire.downloadSpeed(),
        upSpeed: wire.uploadSpeed(),
        downloaded: wire.downloaded,
        uploaded: wire.uploaded,
        type: CONN_LABEL[wire.type] || wire.type || 'TCP'
      })
    }

    // Files
    t.files.forEach((f, i) => {
      base.files.push({
        index: i,
        name: f.name,
        path: f.path,
        length: f.length,
        downloaded: f.downloaded,
        progress: f.progress,
        priority: r.priorities[i] ?? 1
      })
    })

    // Piece map, packed as one byte per piece so the renderer can draw it cheaply.
    if (t.bitfield && t.pieces) {
      const n = t.pieces.length
      const map = new Uint8Array(n)
      let have = 0
      for (let i = 0; i < n; i++) {
        if (t.bitfield.get(i)) { map[i] = 2; have++ }
        else if (t.pieces[i] && t.pieces[i].missing < t.pieces[i].length) map[i] = 1
      }
      base.pieces = Buffer.from(map).toString('base64')
      base.have = have
    }
    return base
  }

  globals () {
    const c = this.client
    // WebTorrent's client exposes live speeds but no cumulative byte counters,
    // so the session totals are summed from the records we already track.
    let downloaded = 0
    let uploaded = 0
    for (const r of this.records.values()) {
      downloaded += r.torrent ? r.torrent.downloaded : r.downloadedBase
      uploaded += r.uploadedBase + (r.torrent ? r.torrent.uploaded : 0)
    }
    return {
      downloadSpeed: c ? c.downloadSpeed : 0,
      uploadSpeed: c ? c.uploadSpeed : 0,
      downloaded,
      uploaded,
      ratio: downloaded > 0 ? uploaded / downloaded : 0,
      progress: c ? c.progress : 0,
      dhtReady: !!c?.dht?.ready,
      dhtNodes: c?.dht?.nodes?.count?.() ?? 0,
      listenPort: c?.torrentPort ?? 0,
      torrentCount: this.records.size,
      altSpeed: this.store.settings.altSpeedEnabled
    }
  }
}

// -------------------------------------------------------------------- helpers

/**
 * Bytes sent per byte received. A torrent seeded from data it already had has
 * downloaded nothing, so its own size stands in as the denominator — otherwise
 * the ratio would be pinned at zero and seeding goals could never be met.
 */
function shareRatio (downloaded, uploaded, length) {
  const base = downloaded > 0 ? downloaded : length
  if (!base) return 0
  return uploaded / base
}

function computeAvailability (t) {
  if (!t.wires?.length || !t.pieces?.length) return t.progress
  const n = t.pieces.length
  let total = 0
  for (const wire of t.wires) {
    if (!wire.peerPieces) continue
    total += bitfieldRatio(wire.peerPieces, n)
  }
  return total
}

function bitfieldRatio (bitfield, n) {
  let count = 0
  for (let i = 0; i < n; i++) if (bitfield.get(i)) count++
  return n ? count / n : 0
}

const CONN_LABEL = {
  tcpIncoming: 'TCP in',
  tcpOutgoing: 'TCP out',
  utpIncoming: 'µTP in',
  utpOutgoing: 'µTP out',
  webrtc: 'WebRTC',
  webSeed: 'Web Seed'
}

const decoder = new TextDecoder('utf8', { fatal: false })

/** Prefers the client's self-reported name, falling back to its peer id. */
function peerClient (wire) {
  const v = wire.peerExtendedHandshake?.v
  if (v) {
    const text = typeof v === 'string' ? v : decoder.decode(v instanceof Uint8Array ? v : new Uint8Array(v))
    const clean = text.replace(/[^\x20-\x7e]/g, '').trim()
    if (clean) return clean
  }
  if (wire.type === 'webSeed') return 'Web Seed'
  return clientFromPeerId(wire.peerId)
}

function peerFlags (wire) {
  const f = []
  if (!wire.peerChoking) f.push('D')      // we can download from them
  if (wire.peerInterested) f.push('U')    // they want data from us
  if (wire.amChoking) f.push('C')
  if (wire.amInterested) f.push('I')
  if (wire.type === 'webSeed') f.push('W')
  if (wire.type === 'utp') f.push('P')
  return f.join('')
}

/** Maps the two-letter Azureus-style prefix in a peer id to a client name. */
const PEER_IDS = {
  UT: 'µTorrent', BT: 'BitTorrent', TR: 'Transmission', qB: 'qBittorrent',
  lt: 'libTorrent', LT: 'libtorrent', AZ: 'Azureus', DE: 'Deluge',
  WW: 'WebTorrent', WD: 'WebTorrent Desktop', UM: 'µTorrent Mac',
  UW: 'µTorrent Web', FD: 'FreeDownloadMgr', TIX: 'Tixati', KT: 'KTorrent'
}

function clientFromPeerId (peerId) {
  if (!peerId) return 'Unknown'
  const id = Buffer.isBuffer(peerId) ? peerId.toString('utf8') : String(peerId)
  const m = id.match(/^-([A-Za-z]{2})(\d{4})-/)
  if (m) {
    const name = PEER_IDS[m[1]] || m[1]
    const v = m[2].split('').join('.').replace(/\.0(?=\.|$)/g, '.0')
    return `${name} ${v}`
  }
  const printable = id.replace(/[^\x20-\x7e]/g, '').slice(0, 12)
  return printable || 'Unknown'
}
