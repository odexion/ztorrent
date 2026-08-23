import { app, BrowserWindow, ipcMain, dialog, Menu, shell, clipboard, Notification, nativeTheme, safeStorage } from 'electron'
import path from 'node:path'
import fs from 'node:fs'
import { fileURLToPath } from 'node:url'
import createTorrent from 'create-torrent'
import parseTorrent from 'parse-torrent'
import { Engine, State } from './engine.js'
import { listInterfaces } from './egress.js'
import { Store, DEFAULT_SETTINGS } from './store.js'

const __dirname = path.dirname(fileURLToPath(import.meta.url))
const ROOT = path.join(__dirname, '..')
const isDev = process.argv.includes('--dev')

// Packaged, sample-torrents lives beside the asar as an extraResource.
const SAMPLES = app.isPackaged
  ? path.join(process.resourcesPath, 'sample-torrents')
  : path.join(ROOT, 'sample-torrents')

// Headless capture hooks, used to verify the UI without screen-recording rights:
//   electron . --add=<file.torrent> --shot=<out.png> --shot-delay=20 --shot-quit
const argVal = (name, fallback) => {
  const hit = process.argv.find(a => a.startsWith(`--${name}=`))
  return hit ? hit.slice(name.length + 3) : fallback
}
const AUTO_ADD = process.argv.filter(a => a.startsWith('--add=')).map(a => a.slice(6))
const SHOT_PATH = argVal('shot', null)
const SHOT_DELAY = Number(argVal('shot-delay', 15))
const SHOT_QUIT = process.argv.includes('--shot-quit')

let win = null
let engine = null
let store = null
let tickTimer = null
let selectedId = null
let pendingOpen = []      // torrents handed to us before the window exists

// Only one instance may own the session directory.
if (!app.requestSingleInstanceLock()) app.quit()

app.on('second-instance', (_e, argv) => {
  if (win) { if (win.isMinimized()) win.restore(); win.focus() }
  handleArgv(argv)
})

// macOS delivers .torrent double-clicks and magnet: links through these.
app.on('open-file', (e, filePath) => { e.preventDefault(); deliver(filePath) })
app.on('open-url', (e, url) => { e.preventDefault(); deliver(url) })

function handleArgv (argv) {
  for (const arg of argv.slice(1)) {
    if (arg.startsWith('-')) continue          // our own flags, not torrents
    if (arg.startsWith('magnet:') || arg.endsWith('.torrent')) deliver(arg)
  }
}

function deliver (source) {
  if (win && !win.isDestroyed()) win.webContents.send('open-torrent', source)
  else pendingOpen.push(source)
}

// --------------------------------------------------------------------- window

function createWindow () {
  const saved = store.data.window || {}
  win = new BrowserWindow({
    width: saved.width || 1180,
    height: saved.height || 760,
    x: saved.x,
    y: saved.y,
    minWidth: 820,
    minHeight: 480,
    title: 'ztorrent',
    backgroundColor: store.settings.theme === 'graphite' ? '#1b1d21' : '#ffffff',
    show: false,
    webPreferences: {
      preload: path.join(__dirname, 'preload.cjs'),
      contextIsolation: true,
      sandbox: true,
      nodeIntegration: false,
      spellcheck: false
    }
  })

  win.loadFile(path.join(ROOT, 'renderer', 'index.html'))
  win.once('ready-to-show', () => {
    win.show()
    for (const src of pendingOpen.splice(0)) deliver(src)
    handleArgv(process.argv)
    if (AUTO_ADD.length) {
      setTimeout(() => AUTO_ADD.forEach(p => engine.add(path.resolve(p))), 600)
    }
    if (SHOT_PATH) {
      const tabs = process.argv.includes('--shot-tabs')
        ? ['general', 'trackers', 'peers', 'pieces', 'files', 'speed', 'logger']
        : [null]
      setTimeout(async () => {
        if (process.env.ZTORRENT_SHOT_EVAL) {
          await win.webContents.executeJavaScript(
            `(() => { ${process.env.ZTORRENT_SHOT_EVAL} })(); null;`)
          await new Promise(r => setTimeout(r, 2200))
        }
        for (const tab of tabs) {
          if (tab) {
            await win.webContents.executeJavaScript(
              `document.querySelector('.tab[data-tab="${tab}"]').click()`)
            await new Promise(r => setTimeout(r, 1600))
          }
          const img = await win.webContents.capturePage()
          const out = tab ? SHOT_PATH.replace(/\.png$/, `-${tab}.png`) : SHOT_PATH
          fs.writeFileSync(out, img.toPNG())
          console.log('[shot] wrote', out)
        }
        if (SHOT_QUIT) app.quit()
      }, SHOT_DELAY * 1000)
    }
  })

  const remember = () => {
    if (!win || win.isDestroyed() || win.isMinimized()) return
    const b = win.getBounds()
    store.set('window', b)
  }
  win.on('resize', remember)
  win.on('move', remember)
  win.on('closed', () => { win = null })

  if (isDev) win.webContents.openDevTools({ mode: 'detach' })
}

// ----------------------------------------------------------------- the ticker

function startTicker () {
  clearInterval(tickTimer)
  tickTimer = setInterval(() => {
    if (!win || win.isDestroyed() || !engine) return
    engine.enforceSeedGoals()
    const rows = engine.snapshot()
    const globals = engine.globals()
    const details = selectedId ? engine.details(selectedId) : null
    win.webContents.send('tick', { rows, globals, details })

    if (process.platform === 'darwin' && store.settings.showSpeedInDock) {
      const active = rows.filter(r => r.state === State.DOWNLOADING)
      if (active.length) {
        const total = active.reduce((s, r) => s + r.size, 0)
        const got = active.reduce((s, r) => s + r.size * r.done, 0)
        app.dock?.setBadge(`${Math.round((got / (total || 1)) * 100)}%`)
        win.setProgressBar(total ? got / total : -1)
      } else {
        app.dock?.setBadge('')
        win.setProgressBar(-1)
      }
    }
  }, 1000)
}

// ------------------------------------------------------------------ IPC: read

ipcMain.handle('snapshot', () => ({ rows: engine.snapshot(), globals: engine.globals() }))
ipcMain.handle('details', (_e, id) => { selectedId = id; return id ? engine.details(id) : null })
ipcMain.handle('settings:get', () => store.settings)
ipcMain.handle('labels:get', () => store.data.labels || [])
ipcMain.handle('log:get', () => engine.logLines)
ipcMain.handle('interfaces:get', () => listInterfaces())
ipcMain.handle('columns:get', () => store.data.columns)
ipcMain.handle('columns:set', (_e, cols) => { store.set('columns', cols); return true })

ipcMain.handle('settings:set', (_e, patch) => {
  // Read before the patch lands, so we can tell the user their proxy is not
  // in force yet rather than letting them assume it is.
  const egressMoved = engine.egressChanged({ ...store.settings, ...patch })
  const next = store.patchSettings(patch)
  if (egressMoved) {
    engine.log(next.proxyEnabled || next.bindInterface
      ? 'Proxy or interface binding changed. It takes effect when ztorrent restarts -- until then traffic goes out as it did before.'
      : 'Proxy and interface binding turned off. They stay in force until ztorrent restarts.')
  }
  engine.applySettings(next)
  if (patch.theme) applyNativeTheme(next.theme)
  win?.webContents.send('settings-changed', next)
  return next
})

// --------------------------------------------------------------- IPC: adding

ipcMain.handle('add:dialog', async () => {
  const res = await dialog.showOpenDialog(win, {
    title: 'Add Torrent',
    buttonLabel: 'Add',
    filters: [{ name: 'Torrent Files', extensions: ['torrent'] }],
    properties: ['openFile', 'multiSelections']
  })
  return res.canceled ? [] : res.filePaths
})

ipcMain.handle('add:paths', async (_e, paths) => {
  const out = []
  for (const p of paths) out.push(await engine.add(p))
  return out
})

ipcMain.handle('add:url', async (_e, url, opts) => engine.add(url, opts || {}))

/** Reads a torrent without adding it, so the Add dialog can show its contents. */
ipcMain.handle('add:inspect', async (_e, source) => {
  try {
    let parsed
    if (typeof source === 'string' && source.startsWith('magnet:')) {
      parsed = await parseTorrent(source)
    } else if (typeof source === 'string' && /^https?:\/\//.test(source)) {
      const res = await fetch(source)
      parsed = await parseTorrent(Buffer.from(await res.arrayBuffer()))
    } else {
      parsed = await parseTorrent(fs.readFileSync(source))
    }
    return {
      ok: true,
      name: parsed.name || parsed.infoHash,
      infoHash: parsed.infoHash,
      length: parsed.length || 0,
      pieceLength: parsed.pieceLength || 0,
      comment: parsed.comment || '',
      createdBy: parsed.createdBy || '',
      created: parsed.created ? new Date(parsed.created).getTime() : 0,
      private: !!parsed.private,
      announce: parsed.announce || [],
      files: (parsed.files || []).map((f, i) => ({
        index: i, name: f.name, path: f.path, length: f.length
      }))
    }
  } catch (err) {
    return { ok: false, error: err.message }
  }
})

ipcMain.handle('add:confirmed', async (_e, source, opts) => engine.add(source, opts || {}))

// ------------------------------------------------------------- IPC: commands

const each = (ids, fn) => (Array.isArray(ids) ? ids : [ids]).forEach(fn)

ipcMain.handle('cmd:start', (_e, ids) => each(ids, id => engine.startTorrent(id)))
ipcMain.handle('cmd:pause', (_e, ids) => each(ids, id => engine.pauseTorrent(id)))
ipcMain.handle('cmd:stop', (_e, ids) => each(ids, id => engine.stopTorrent(id)))
ipcMain.handle('cmd:recheck', (_e, ids) => each(ids, id => engine.recheck(id)))
ipcMain.handle('cmd:move', (_e, id, delta) => engine.moveQueue(id, delta))
ipcMain.handle('cmd:label', (_e, ids, label) => each(ids, id => engine.setLabel(id, label)))
ipcMain.handle('cmd:sequential', (_e, id, on) => engine.setSequential(id, on))
ipcMain.handle('cmd:filePriority', (_e, id, i, p) => engine.setFilePriority(id, i, p))
ipcMain.handle('cmd:addTracker', (_e, id, url) => engine.addTracker(id, url))
ipcMain.handle('cmd:addPeer', (_e, id, addr) => engine.addPeer(id, addr))
ipcMain.handle('cmd:reannounce', (_e, id) => engine.reannounce(id))

ipcMain.handle('cmd:altSpeed', () => {
  const on = !store.settings.altSpeedEnabled
  store.patchSettings({ altSpeedEnabled: on })
  engine.setAltSpeed(on)
  return on
})

ipcMain.handle('cmd:remove', async (_e, ids, deleteData) => {
  const list = Array.isArray(ids) ? ids : [ids]
  if (store.settings.confirmOnDelete) {
    const names = list.map(id => engine.records.get(id)?.name).filter(Boolean)
    const { response } = await dialog.showMessageBox(win, {
      type: 'warning',
      buttons: ['Cancel', deleteData ? 'Delete Data' : 'Remove'],
      defaultId: 1,
      cancelId: 0,
      message: deleteData
        ? `Remove ${list.length === 1 ? `"${names[0]}"` : `${list.length} torrents`} and delete the downloaded data?`
        : `Remove ${list.length === 1 ? `"${names[0]}"` : `${list.length} torrents`} from the list?`,
      detail: deleteData
        ? 'The files will be moved to the trash of no return — this cannot be undone.'
        : 'The downloaded files will be left on disk.'
    })
    if (response !== 1) return false
  }
  list.forEach(id => engine.removeTorrent(id, deleteData))
  return true
})

// ------------------------------------------------------------ IPC: utilities

ipcMain.handle('util:chooseFolder', async (_e, title) => {
  const res = await dialog.showOpenDialog(win, {
    title: title || 'Choose Folder', properties: ['openDirectory', 'createDirectory']
  })
  return res.canceled ? null : res.filePaths[0]
})

ipcMain.handle('util:chooseFile', async (_e, title) => {
  const res = await dialog.showOpenDialog(win, {
    title: title || 'Choose File', properties: ['openFile']
  })
  return res.canceled ? null : res.filePaths[0]
})

ipcMain.handle('util:saveFile', async (_e, name, filters) => {
  const res = await dialog.showSaveDialog(win, { defaultPath: name, filters })
  return res.canceled ? null : res.filePath
})

ipcMain.handle('util:clipboard', (_e, text) => { clipboard.writeText(String(text)); return true })

function resolveTarget (id, fileIndex) {
  const r = engine.records.get(id)
  if (!r) return null
  if (typeof fileIndex === 'number' && r.torrent?.files[fileIndex]) {
    return path.join(r.savePath, r.torrent.files[fileIndex].path)
  }
  return path.join(r.savePath, r.name)
}

ipcMain.handle('util:reveal', (_e, id, fileIndex) => {
  const target = resolveTarget(id, fileIndex)
  if (!target) return false
  if (fs.existsSync(target)) shell.showItemInFolder(target)
  else shell.openPath(engine.records.get(id).savePath)
  return true
})

ipcMain.handle('util:open', (_e, id, fileIndex) => {
  const target = resolveTarget(id, fileIndex)
  if (target && fs.existsSync(target)) shell.openPath(target)
  return true
})

ipcMain.handle('util:saveTorrent', (_e, id, dest) => {
  const r = engine.records.get(id)
  if (!r?.torrentFile) return { ok: false, error: 'Torrent metadata is not available yet.' }
  try {
    fs.writeFileSync(dest, Buffer.from(r.torrentFile, 'base64'))
    engine.log(`Saved "${r.name}" as ${dest}`)
    return { ok: true }
  } catch (err) {
    return { ok: false, error: err.message }
  }
})

ipcMain.handle('util:badge', (_e, text) => { app.dock?.setBadge(text || ''); return true })
ipcMain.handle('util:progress', (_e, v) => { win?.setProgressBar(v); return true })

/** Builds the torrent-list / file-list right-click menus natively. */
ipcMain.handle('util:contextMenu', (_e, kind, payload) => new Promise(resolve => {
  let template = []
  const pick = action => () => resolve(action)

  if (kind === 'torrent') {
    const single = payload.count === 1
    template = [
      { label: 'Start', click: pick('start') },
      { label: 'Pause', click: pick('pause') },
      { label: 'Stop', click: pick('stop') },
      { type: 'separator' },
      { label: 'Force Re-Check', click: pick('recheck') },
      { label: 'Update Tracker', enabled: single, click: pick('reannounce') },
      { type: 'separator' },
      { label: 'Remove', click: pick('remove') },
      { label: 'Remove And Delete Data…', click: pick('remove-data') },
      { type: 'separator' },
      {
        label: 'Bandwidth Allocation',
        submenu: [
          { label: 'Move Up Queue', enabled: single, click: pick('queue-up') },
          { label: 'Move Down Queue', enabled: single, click: pick('queue-down') },
          { type: 'separator' },
          {
            label: 'Download Sequentially',
            type: 'checkbox',
            checked: !!payload.sequential,
            enabled: single,
            click: pick('toggle-sequential')
          }
        ]
      },
      {
        label: 'Labels',
        submenu: [
          { label: 'Remove Label', click: pick('label:') },
          { type: 'separator' },
          ...(store.data.labels || []).map(l => ({
            label: l, type: 'radio', checked: payload.label === l, click: pick('label:' + l)
          })),
          { type: 'separator' },
          { label: 'New Label…', click: pick('label-new') }
        ]
      },
      { type: 'separator' },
      { label: 'Open Containing Folder', enabled: single, click: pick('reveal') },
      { label: 'Copy Magnet URI', enabled: single, click: pick('magnet') },
      { label: 'Save .torrent As…', enabled: single, click: pick('save-torrent') },
      { type: 'separator' },
      { label: 'Properties…', enabled: single, click: pick('properties') }
    ]
  } else if (kind === 'file') {
    template = [
      { label: 'Open', click: pick('open') },
      { label: 'Show in Finder', click: pick('reveal') },
      { type: 'separator' },
      { label: 'High Priority', type: 'radio', checked: payload.priority === 2, click: pick('prio:2') },
      { label: 'Normal Priority', type: 'radio', checked: payload.priority === 1, click: pick('prio:1') },
      { label: "Don't Download", type: 'radio', checked: payload.priority === 0, click: pick('prio:0') }
    ]
  } else if (kind === 'tracker') {
    template = [
      { label: 'Add Tracker…', click: pick('add') },
      { label: 'Update Tracker', click: pick('update') },
      { type: 'separator' },
      { label: 'Copy Tracker URL', click: pick('copy') }
    ]
  } else if (kind === 'peer') {
    template = [
      { label: 'Add Peer…', click: pick('add') },
      { label: 'Copy Peer Address', click: pick('copy') }
    ]
  } else if (kind === 'columns') {
    template = payload.columns.map(c => ({
      label: c.label === '#' ? 'Order (#)' : c.label,
      type: 'checkbox',
      checked: c.on,
      enabled: c.key !== '#' && c.key !== 'name',
      click: pick('col:' + c.key)
    }))
  }

  const menu = Menu.buildFromTemplate(template)
  menu.popup({ window: win, callback: () => resolve(null) })
}))

// -------------------------------------------------------- IPC: create torrent

ipcMain.handle('util:createTorrent', async (_e, opts) => {
  return new Promise(resolve => {
    const input = opts.inputPath
    const settings = {
      name: opts.name || path.basename(input),
      comment: opts.comment || undefined,
      createdBy: 'ztorrent 1.0.0',
      private: !!opts.private,
      pieceLength: opts.pieceLength > 0 ? opts.pieceLength : undefined,
      announceList: (opts.trackers || [])
        .map(t => t.trim()).filter(Boolean).map(t => [t]),
      urlList: (opts.webSeeds || []).map(t => t.trim()).filter(Boolean)
    }
    createTorrent(input, settings, (err, buf) => {
      if (err) return resolve({ ok: false, error: err.message })
      try {
        fs.writeFileSync(opts.outputPath, buf)
      } catch (e) {
        return resolve({ ok: false, error: e.message })
      }
      engine.log(`Created torrent "${settings.name}" -> ${opts.outputPath}`)
      if (opts.startSeeding) {
        engine.add(buf, { savePath: path.dirname(input), paused: false })
      }
      resolve({ ok: true, path: opts.outputPath, size: buf.length })
    })
  })
})

// ------------------------------------------------------------------ app menu

function buildMenu () {
  const send = (action, arg) => () => win?.webContents.send('menu', { action, arg })
  const isMac = process.platform === 'darwin'

  // The application menu, Services and hide/unhide are macOS-only concepts;
  // elsewhere Preferences and Quit belong in File.
  const appMenu = {
      label: 'ztorrent',
      submenu: [
        { label: 'About ztorrent', click: showAbout },
        { type: 'separator' },
        { label: 'Preferences…', accelerator: 'CmdOrCtrl+,', click: send('preferences') },
        { type: 'separator' },
        { role: 'services' },
        { type: 'separator' },
        { role: 'hide' }, { role: 'hideOthers' }, { role: 'unhide' },
        { type: 'separator' },
        { role: 'quit' }
      ]
  }

  const template = [
    ...(isMac ? [appMenu] : []),
    {
      label: 'File',
      submenu: [
        { label: 'Add Torrent…', accelerator: 'CmdOrCtrl+O', click: send('add-file') },
        { label: 'Add Torrent from URL…', accelerator: 'CmdOrCtrl+U', click: send('add-url') },
        { type: 'separator' },
        { label: 'Create New Torrent…', accelerator: 'CmdOrCtrl+N', click: send('create-torrent') },
        { type: 'separator' },
        { role: 'close' },
        ...(isMac
          ? []
          : [{ type: 'separator' },
             { label: 'Preferences…', accelerator: 'CmdOrCtrl+,', click: send('preferences') },
             { type: 'separator' },
             { role: 'quit', label: 'Exit' }])
      ]
    },
    {
      label: 'Edit',
      submenu: [
        { role: 'undo' }, { role: 'redo' }, { type: 'separator' },
        { role: 'cut' }, { role: 'copy' }, { role: 'paste' }, { role: 'selectAll' },
        { type: 'separator' },
        { label: 'Find', accelerator: 'CmdOrCtrl+F', click: send('focus-search') }
      ]
    },
    {
      label: 'Torrent',
      submenu: [
        { label: 'Start', accelerator: 'CmdOrCtrl+R', click: send('start') },
        { label: 'Pause', accelerator: 'CmdOrCtrl+P', click: send('pause') },
        { label: 'Stop', accelerator: 'CmdOrCtrl+.', click: send('stop') },
        { type: 'separator' },
        { label: 'Force Re-Check', accelerator: 'CmdOrCtrl+E', click: send('recheck') },
        { label: 'Update Tracker', accelerator: 'CmdOrCtrl+T', click: send('reannounce') },
        { type: 'separator' },
        { label: 'Move Up Queue', accelerator: 'CmdOrCtrl+Up', click: send('queue-up') },
        { label: 'Move Down Queue', accelerator: 'CmdOrCtrl+Down', click: send('queue-down') },
        { type: 'separator' },
        { label: 'Remove', accelerator: 'CmdOrCtrl+Backspace', click: send('remove') },
        { label: 'Remove And Delete Data…', accelerator: 'CmdOrCtrl+Shift+Backspace', click: send('remove-data') },
        { type: 'separator' },
        { label: 'Copy Magnet URI', accelerator: 'CmdOrCtrl+Shift+C', click: send('magnet') },
        { label: 'Open Containing Folder', accelerator: 'CmdOrCtrl+Shift+O', click: send('reveal') }
      ]
    },
    {
      label: 'Options',
      submenu: [
        {
          label: 'Alternate Speed Limits',
          type: 'checkbox',
          accelerator: 'CmdOrCtrl+Shift+L',
          checked: store.settings.altSpeedEnabled,
          click: mi => {
            store.patchSettings({ altSpeedEnabled: mi.checked })
            engine.setAltSpeed(mi.checked)
            win?.webContents.send('settings-changed', store.settings)
          }
        },
        { type: 'separator' },
        { label: 'Preferences…', accelerator: 'CmdOrCtrl+Alt+,', click: send('preferences') }
      ]
    },
    {
      label: 'View',
      submenu: [
        { label: 'Show Sidebar', type: 'checkbox', checked: true, accelerator: 'CmdOrCtrl+1', click: send('toggle-sidebar') },
        { label: 'Show Detailed Info', type: 'checkbox', checked: true, accelerator: 'CmdOrCtrl+2', click: send('toggle-detail') },
        { label: 'Show Status Bar', type: 'checkbox', checked: true, accelerator: 'CmdOrCtrl+3', click: send('toggle-statusbar') },
        { type: 'separator' },
        {
          label: 'Appearance',
          submenu: [
            { label: 'Light', type: 'radio', checked: store.settings.theme === 'classic', click: () => setTheme('classic') },
            { label: 'Dark', type: 'radio', checked: store.settings.theme === 'graphite', click: () => setTheme('graphite') }
          ]
        },
        { type: 'separator' },
        { role: 'reload' }, { role: 'toggleDevTools' },
        { type: 'separator' },
        { role: 'togglefullscreen' }
      ]
    },
    {
      label: 'Window',
      submenu: isMac
        ? [{ role: 'minimize' }, { role: 'zoom' }, { type: 'separator' }, { role: 'front' }]
        : [{ role: 'minimize' }, { role: 'close' }]
    },
    {
      role: 'help',
      submenu: [
        { label: 'ztorrent Help', click: showAbout },
        { label: 'Sample Torrents Folder', click: () => shell.openPath(SAMPLES) }
      ]
    }
  ]
  Menu.setApplicationMenu(Menu.buildFromTemplate(template))
}

function setTheme (theme) {
  store.patchSettings({ theme })
  applyNativeTheme(theme)
  win?.webContents.send('settings-changed', store.settings)
}

/** Keep Electron's own chrome in step with the renderer's theme. */
function applyNativeTheme (theme) {
  const dark = theme === 'graphite'
  nativeTheme.themeSource = dark ? 'dark' : 'light'
  win?.setBackgroundColor(dark ? '#1b1d21' : '#ffffff')
}

function showAbout () {
  dialog.showMessageBox(win, {
    type: 'info',
    message: 'ztorrent 1.0.0',
    detail: 'A BitTorrent client for macOS.\n\nBuilt on WebTorrent — DHT, PEX, LSD, µTP,\nHTTP/UDP trackers and web seeds.',
    buttons: ['OK']
  })
}

// ------------------------------------------------------------------ bootstrap

/**
 * Keeps the proxy password out of the settings file. safeStorage is backed by
 * the Keychain on macOS and DPAPI on Windows; where it is unavailable (some
 * Linux desktops) we hand back nothing and the value stays plaintext, as it
 * was before -- losing the password would be worse than storing it.
 */
function makeSecretCodec () {
  if (!safeStorage.isEncryptionAvailable()) {
    console.warn('[store] safeStorage unavailable; secrets stay in plain text')
    return null
  }
  return {
    encrypt: value => safeStorage.encryptString(String(value)).toString('base64'),
    decrypt: sealed => safeStorage.decryptString(Buffer.from(String(sealed), 'base64'))
  }
}

app.whenReady().then(() => {
  // Packaged builds get their icon from the bundle; unpackaged runs need it set.
  if (process.platform === 'darwin' && !app.isPackaged) {
    const iconPath = path.join(ROOT, 'build', 'icon.png')
    if (fs.existsSync(iconPath)) app.dock?.setIcon(iconPath)
  }
  store = new Store(app.getPath('userData'), makeSecretCodec())
  engine = new Engine(store)

  applyNativeTheme(store.settings.theme)

  engine.on('log', line => win?.webContents.send('log', line))
  engine.on('changed', () => win?.webContents.send('changed'))
  engine.on('complete', ({ name, path: savePath }) => {
    if (!store.settings.notifyOnComplete || !Notification.isSupported()) return
    const n = new Notification({
      title: 'Download complete',
      body: name,
      silent: false
    })
    n.on('click', () => shell.openPath(savePath))
    n.show()
  })

  engine.start()
  buildMenu()
  createWindow()
  startTicker()

  app.on('activate', () => {
    if (BrowserWindow.getAllWindows().length === 0) createWindow()
  })
})

app.on('window-all-closed', () => { if (process.platform !== 'darwin') app.quit() })

let quitting = false
app.on('before-quit', async e => {
  if (quitting) return
  e.preventDefault()
  quitting = true
  clearInterval(tickTimer)
  try { await engine?.destroy() } catch { /* shutting down anyway */ }
  app.exit(0)
})
