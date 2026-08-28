import { app, net, shell } from 'electron'
import { EventEmitter } from 'node:events'
import { execFile, spawn } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'
import os from 'node:os'

/**
 * Checks GitHub for a newer release, downloads the build for this platform, and
 * swaps it in on the next launch.
 *
 * These builds are unsigned, so Squirrel and the platform update services are
 * out -- both insist on a signature before they will replace an application.
 * What is left is what the curl installer already does by hand: fetch the same
 * artifact from the same release and copy it over the installed one. The swap
 * itself cannot happen while the app is holding its own files open, so it is
 * handed to a small detached script that waits for this process to exit, moves
 * the old copy aside, puts the new one in place, and launches it again. Moving
 * rather than deleting means a failed copy can be rolled back: the worst case
 * is the old version starting up again, never no version at all.
 *
 * Nothing is downloaded until a check has found a genuinely newer version, and
 * nothing is swapped until the user asks for it.
 */

const REPO = 'odexion/ztorrent'
const CHECK_EVERY = 6 * 60 * 60 * 1000   // six hours
const FIRST_CHECK_AFTER = 20 * 1000      // let the app finish starting first

/** 'idle' | 'checking' | 'available' | 'downloading' | 'staging' | 'ready' | 'error' */
export const UpdateState = {
  IDLE: 'idle',
  CHECKING: 'checking',
  AVAILABLE: 'available',
  DOWNLOADING: 'downloading',
  STAGING: 'staging',
  READY: 'ready',
  ERROR: 'error'
}

/**
 * Compares two dotted versions numerically. A version carrying a prerelease
 * suffix (1.2.0-beta.1) sorts below the same version without one, which is the
 * only part of the semver ordering rules this needs to get right.
 */
export function compareVersions (a, b) {
  const split = v => {
    const [core, pre] = String(v).replace(/^v/, '').split('-')
    return { parts: core.split('.').map(n => parseInt(n, 10) || 0), pre: pre || '' }
  }
  const x = split(a)
  const y = split(b)
  for (let i = 0; i < Math.max(x.parts.length, y.parts.length); i++) {
    const d = (x.parts[i] || 0) - (y.parts[i] || 0)
    if (d) return d < 0 ? -1 : 1
  }
  if (x.pre === y.pre) return 0
  if (!x.pre) return 1        // a release outranks its own prereleases
  if (!y.pre) return -1
  return x.pre < y.pre ? -1 : 1
}

/**
 * The artifact this platform can actually install, named the way
 * electron-builder names it. Each target spells its architecture in its own
 * convention -- the dmg says x64, the AppImage x86_64, the deb amd64 -- so
 * every spelling is tried rather than the one that happens to be right today.
 * This mirrors scripts/install.sh; the two must agree on what to pick.
 */
function wantedAsset () {
  const arches = process.arch === 'arm64'
    ? ['arm64', 'aarch64']
    : ['x64', 'x86_64', 'amd64']

  if (process.platform === 'darwin') return { os: 'mac', ext: 'dmg', arches }
  if (process.platform === 'win32') return { os: 'win', ext: 'exe', arches }
  // Only the AppImage is a single file we can drop in place. A .deb needs the
  // package manager, and root, so a Linux install that did not come from an
  // AppImage is told about the release rather than handed one.
  if (process.env.APPIMAGE) return { os: 'linux', ext: 'AppImage', arches }
  return null
}

/** Runs a command to completion; rejects on a non-zero exit. */
function run (cmd, args) {
  return new Promise((resolve, reject) => {
    execFile(cmd, args, { maxBuffer: 8 << 20 }, (err, stdout) =>
      err ? reject(err) : resolve(String(stdout || '')))
  })
}

export class Updater extends EventEmitter {
  constructor (dir, { repo = REPO } = {}) {
    super()
    this.repo = process.env.ZTORRENT_UPDATE_REPO || repo
    this.dir = dir                       // <userData>/updates
    this.staged = path.join(dir, 'staged')
    this.timer = null

    this.current = process.env.ZTORRENT_UPDATE_PRETEND_VERSION || app.getVersion()
    this.status = {
      state: UpdateState.IDLE,
      version: null,
      url: null,
      size: 0,
      received: 0,
      file: null,
      error: null,
      // Linux outside an AppImage, or any platform with no matching artifact:
      // the release is still worth telling the user about, but this process
      // cannot install it.
      installable: wantedAsset() !== null
    }
  }

  // ------------------------------------------------------------------ state

  state () { return { ...this.status, current: this.current } }

  _set (patch) {
    Object.assign(this.status, patch)
    this.emit('state', this.state())
  }

  // ------------------------------------------------------------------ timer

  /** Leftovers from an update that was downloaded but never applied. */
  reset () {
    try { fs.rmSync(this.dir, { recursive: true, force: true }) } catch { /* nothing staged */ }
  }

  start () {
    clearTimeout(this.timer)
    this.timer = setTimeout(() => this._tick(), FIRST_CHECK_AFTER)
  }

  stop () { clearTimeout(this.timer); this.timer = null }

  _tick () {
    this.check().catch(() => { /* reported through state */ })
    clearTimeout(this.timer)
    this.timer = setTimeout(() => this._tick(), CHECK_EVERY)
  }

  /**
   * Picks up a build staged by an earlier run. Downloading 100MB again because
   * the user quit before restarting would be a poor trade, so a staged update
   * survives a normal quit and the pill comes back offering the restart. A
   * staging directory for a version we are already running is the leftover of
   * an update that went through, and is thrown away.
   */
  resume () {
    let staged = null
    try {
      staged = JSON.parse(fs.readFileSync(path.join(this.dir, 'staged.json'), 'utf8'))
    } catch { return }

    const newer = staged?.version && compareVersions(staged.version, this.current) > 0
    if (!newer || !staged.payload || !fs.existsSync(staged.payload)) {
      this.reset()
      return
    }
    this._set({
      state: UpdateState.READY,
      version: staged.version,
      file: staged.payload
    })
  }

  // ------------------------------------------------------------------ check

  async _api (url) {
    const res = await net.fetch(url, {
      headers: {
        'User-Agent': `ztorrent/${this.current}`,
        Accept: 'application/vnd.github+json'
      }
    })
    if (!res.ok) throw new Error(`GitHub returned ${res.status}`)
    return res.json()
  }

  /**
   * Looks for a newer release. Resolves to the state either way -- a failed
   * check is a fact about the network, not something to interrupt anyone over,
   * so it is recorded and left for the caller to show or ignore.
   */
  async check ({ manual = false } = {}) {
    if (this.status.state === UpdateState.DOWNLOADING ||
        this.status.state === UpdateState.STAGING) return this.state()

    // Already holding a build that is ready to go: nothing to look for.
    if (this.status.state === UpdateState.READY && !manual) return this.state()

    // Remembered across the CHECKING state below, which overwrites both.
    const staged = this.status.state === UpdateState.READY
      ? { version: this.status.version, file: this.status.file }
      : null

    this._set({ state: UpdateState.CHECKING, error: null })
    try {
      const feed = process.env.ZTORRENT_UPDATE_FEED ||
        `https://api.github.com/repos/${this.repo}/releases/latest`
      const release = await this._api(feed)
      const version = String(release.tag_name || '').replace(/^v/, '')
      if (!version) throw new Error('the release has no tag')

      if (compareVersions(version, this.current) <= 0) {
        this._set({ state: UpdateState.IDLE, version: null, url: null, size: 0 })
        return this.state()
      }

      // The build we already staged is the one the release is offering. Say so
      // again rather than dropping back to AVAILABLE, which would throw the
      // payload away and ask for the same download a second time.
      if (staged && staged.version === version) {
        this._set({ state: UpdateState.READY, version, file: staged.file })
        return this.state()
      }
      // Staged, but the release has moved on since: that payload is no longer
      // the one to install, so it goes rather than sitting there taking room.
      if (staged) this.reset()

      const want = wantedAsset()
      const assets = Array.isArray(release.assets) ? release.assets : []
      let asset = null
      if (want) {
        for (const a of want.arches) {
          asset = assets.find(x => String(x.name).endsWith(`-${want.os}-${a}.${want.ext}`))
          if (asset) break
        }
      }

      this._set({
        state: UpdateState.AVAILABLE,
        version,
        url: asset?.browser_download_url || null,
        size: asset?.size || 0,
        received: 0,
        file: null,
        installable: Boolean(asset)
      })
      return this.state()
    } catch (err) {
      // A check that could not reach the network must not cost a build that is
      // already staged and waiting: keep offering the restart, and carry the
      // error alongside it for whoever asked.
      if (staged) {
        this._set({
          state: UpdateState.READY,
          version: staged.version,
          file: staged.file,
          error: err.message
        })
      } else {
        this._set({ state: UpdateState.ERROR, error: err.message })
      }
      return this.state()
    }
  }

  // --------------------------------------------------------------- download

  async download () {
    if (this.status.state !== UpdateState.AVAILABLE) return this.state()
    if (!this.status.url) return this.state()

    const version = this.status.version
    fs.mkdirSync(this.dir, { recursive: true })
    const name = path.basename(new URL(this.status.url).pathname)
    const part = path.join(this.dir, `${name}.part`)
    const dest = path.join(this.dir, name)

    this._set({ state: UpdateState.DOWNLOADING, received: 0, error: null })
    try {
      const res = await net.fetch(this.status.url, {
        headers: { 'User-Agent': `ztorrent/${this.current}` }
      })
      if (!res.ok) throw new Error(`download returned ${res.status}`)

      const total = Number(res.headers.get('content-length')) || this.status.size
      if (total) this._set({ size: total })

      const out = fs.createWriteStream(part)
      let received = 0
      let painted = 0
      for await (const chunk of res.body) {
        out.write(Buffer.from(chunk))
        received += chunk.length
        // The renderer redraws on every push; at a megabyte a chunk that is far
        // more often than a progress readout can usefully change.
        if (received - painted > 262144) {
          painted = received
          this._set({ received })
        }
      }
      await new Promise((resolve, reject) => out.end(err => err ? reject(err) : resolve()))

      if (total && received !== total) {
        throw new Error(`expected ${total} bytes, got ${received}`)
      }
      fs.renameSync(part, dest)
      this._set({ received, file: dest })

      await this._stage(dest, version)
      return this.state()
    } catch (err) {
      try { fs.rmSync(part, { force: true }) } catch { /* already gone */ }
      this._set({ state: UpdateState.ERROR, error: err.message })
      return this.state()
    }
  }

  /**
   * Turns the downloaded artifact into something the swap can move in one step.
   * The .app comes out of the disk image here rather than at apply time, while
   * the app is still running and a failure is still recoverable -- once we have
   * quit there is no UI left to report one in.
   */
  async _stage (file, version) {
    this._set({ state: UpdateState.STAGING })
    fs.rmSync(this.staged, { recursive: true, force: true })
    fs.mkdirSync(this.staged, { recursive: true })

    let payload = file
    if (process.platform === 'darwin') {
      payload = await this._extractApp(file)
      fs.rmSync(file, { force: true })     // the image is 100MB+ and done with
    } else if (process.platform === 'linux') {
      payload = path.join(this.staged, 'ztorrent.AppImage')
      fs.copyFileSync(file, payload)
      fs.chmodSync(payload, 0o755)
      fs.rmSync(file, { force: true })
    }

    fs.writeFileSync(
      path.join(this.dir, 'staged.json'),
      JSON.stringify({ version, payload }, null, 2))

    this._set({ state: UpdateState.READY, version, file: payload })
  }

  /** Mounts the .dmg, copies the bundle out of it, and unmounts it again. */
  async _extractApp (dmg) {
    await this._detachImage(dmg)

    const mnt = fs.mkdtempSync(path.join(os.tmpdir(), 'ztorrent-update-'))
    try {
      await run('hdiutil', [
        'attach', '-nobrowse', '-readonly', '-noautoopen', '-mountpoint', mnt, dmg
      ])
      const src = fs.readdirSync(mnt).find(n => n.endsWith('.app'))
      if (!src) throw new Error('no application inside the disk image')

      const dest = path.join(this.staged, src)
      // ditto, not cp -R: it is the copier that keeps a bundle's symlinks,
      // permissions and extended attributes intact.
      await run('ditto', [path.join(mnt, src), dest])
      return dest
    } finally {
      // Detached before the caller deletes the image, and before we hand back
      // either a path or an error -- an image left attached is what breaks the
      // next attempt.
      await run('hdiutil', ['detach', mnt, '-force']).catch(() => {})
      try { fs.rmdirSync(mnt) } catch { /* the detach took it */ }
    }
  }

  /**
   * Ejects anything still holding this image. A run killed between attach and
   * detach leaves it mounted, and every later attach of the same file then
   * fails with "Resource busy" -- which would wedge updates for good rather
   * than for one attempt.
   */
  async _detachImage (dmg) {
    let info = ''
    try { info = await run('hdiutil', ['info']) } catch { return }

    const wanted = path.resolve(dmg)
    for (const block of info.split(/^=+$/m)) {
      const image = block.match(/^image-path\s*:\s*(.+)$/m)?.[1]?.trim()
      if (!image || path.resolve(image) !== wanted) continue
      const dev = block.match(/^(\/dev\/disk\d+)\s/m)?.[1]
      if (dev) await run('hdiutil', ['detach', dev, '-force']).catch(() => {})
    }
  }

  // ------------------------------------------------------------------ apply

  /** Where the running application lives, as something the swap can replace. */
  installTarget () {
    if (process.platform === 'darwin') {
      // .../ztorrent.app/Contents/MacOS/ztorrent -> .../ztorrent.app
      const exe = app.getPath('exe')
      const i = exe.indexOf('.app/')
      return i === -1 ? null : exe.slice(0, i + 4)
    }
    if (process.platform === 'linux') return process.env.APPIMAGE || null
    return app.getPath('exe')
  }

  /**
   * Writes the hand-off script and starts it detached, so it outlives us and
   * can replace files this process still has open. Returns false when there is
   * nothing staged to apply.
   */
  applyAndRestart () {
    // An unpackaged run is executing out of Electron.app, and installTarget()
    // would happily point the swap at it. Never replace the development host.
    if (!app.isPackaged) return false
    if (this.status.state !== UpdateState.READY || !this.status.file) return false

    const payload = this.status.file
    const target = this.installTarget()
    if (!target) return false

    if (process.platform === 'win32') {
      // The NSIS installer already knows how to stop, replace and restart the
      // app; handing it the job beats reimplementing it.
      spawn(payload, ['/S', '--force-run'], { detached: true, stdio: 'ignore' }).unref()
      return true
    }

    const script = path.join(this.dir, 'apply.sh')
    fs.writeFileSync(script, this._script(payload, target), { mode: 0o755 })
    spawn('/bin/sh', [script, String(process.pid)], {
      detached: true,
      stdio: 'ignore'
    }).unref()
    return true
  }

  _script (payload, target) {
    const q = s => `'${String(s).replace(/'/g, `'\\''`)}'`

    const swap = process.platform === 'darwin'
      ? `
backup=${q(target)}.old.$$
if mv ${q(target)} "$backup" 2>/dev/null; then
  if ditto ${q(payload)} ${q(target)}; then
    /usr/bin/xattr -dr com.apple.quarantine ${q(target)} 2>/dev/null || true
    rm -rf "$backup"
  else
    # Put back exactly what was there. A failed update must not cost anyone
    # the version they already had.
    rm -rf ${q(target)}
    mv "$backup" ${q(target)}
  fi
fi
open ${q(target)}
`
      : `
if cp -f ${q(payload)} ${q(target)}.new 2>/dev/null; then
  chmod +x ${q(target)}.new
  mv -f ${q(target)}.new ${q(target)}
fi
${q(target)} >/dev/null 2>&1 &
`

    return `#!/bin/sh
# Written by ztorrent to finish an update. Safe to delete.
#
# Waits for the running copy to exit -- it cannot be replaced while it holds its
# own files open -- then swaps the new build in and starts it again.
pid="\${1:-}"
# A kill -0 aimed at pid 0 signals our own process group and always succeeds,
# so anything that is not a real pid has to skip the wait rather than sit
# through the whole of it.
case "$pid" in ''|0|*[!0-9]*) pid="" ;; esac

i=0
while [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null && [ "$i" -lt 600 ]; do
  sleep 0.1
  i=$((i + 1))
done
${swap}
rm -rf ${q(this.staged)} ${q(path.join(this.dir, 'staged.json'))}
rm -f "$0"
`
  }

  /** The release page, for platforms this cannot install for itself. */
  openReleasePage () {
    const tag = this.status.version ? `tag/v${this.status.version}` : 'latest'
    return shell.openExternal(`https://github.com/${this.repo}/releases/${tag}`)
  }
}
