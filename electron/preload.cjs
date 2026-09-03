const { contextBridge, ipcRenderer, webUtils } = require('electron')

/**
 * The only surface the renderer sees. Everything is a named channel so the
 * renderer never gets a handle on Node or on the WebTorrent client itself.
 */
const invoke = (channel, ...args) => ipcRenderer.invoke(channel, ...args)

contextBridge.exposeInMainWorld('ztorrent', {
  // ---- environment
  platform: process.platform,

  // ---- queries
  getSnapshot: () => invoke('snapshot'),
  getDetails: id => invoke('details', id),
  getSettings: () => invoke('settings:get'),
  setSettings: patch => invoke('settings:set', patch),
  getLabels: () => invoke('labels:get'),
  getLabelStyles: () => invoke('labels:styles'),
  setLabelStyle: (name, style) => invoke('labels:setStyle', name, style),
  getLog: () => invoke('log:get'),
  getInterfaces: () => invoke('interfaces:get'),
  getColumns: () => invoke('columns:get'),
  setColumns: cols => invoke('columns:set', cols),

  // ---- application
  getVersion: () => invoke('app:version'),
  showAbout: () => invoke('app:about'),

  // ---- updates
  getUpdate: () => invoke('update:get'),
  checkForUpdate: () => invoke('update:check'),
  downloadUpdate: () => invoke('update:download'),
  applyUpdate: () => invoke('update:apply'),
  openReleasePage: () => invoke('update:openPage'),

  // ---- adding
  addTorrentDialog: () => invoke('add:dialog'),
  addTorrentPaths: paths => invoke('add:paths', paths),
  addTorrentURL: (url, opts) => invoke('add:url', url, opts),
  inspectTorrent: source => invoke('add:inspect', source),
  addInspected: (source, opts) => invoke('add:confirmed', source, opts),

  // ---- commands
  start: ids => invoke('cmd:start', ids),
  pause: ids => invoke('cmd:pause', ids),
  stop: ids => invoke('cmd:stop', ids),
  remove: (ids, deleteData) => invoke('cmd:remove', ids, deleteData),
  recheck: ids => invoke('cmd:recheck', ids),
  moveQueue: (id, delta) => invoke('cmd:move', id, delta),
  setLabel: (ids, label) => invoke('cmd:label', ids, label),
  setSequential: (id, on) => invoke('cmd:sequential', id, on),
  setFilePriority: (id, index, priority) => invoke('cmd:filePriority', id, index, priority),
  addTracker: (id, url) => invoke('cmd:addTracker', id, url),
  addPeer: (id, addr) => invoke('cmd:addPeer', id, addr),
  reannounce: id => invoke('cmd:reannounce', id),
  toggleAltSpeed: () => invoke('cmd:altSpeed'),

  // ---- utilities
  createTorrent: opts => invoke('util:createTorrent', opts),
  chooseFolder: title => invoke('util:chooseFolder', title),
  chooseFile: title => invoke('util:chooseFile', title),
  saveFileDialog: (name, filters) => invoke('util:saveFile', name, filters),
  revealInFinder: (id, fileIndex) => invoke('util:reveal', id, fileIndex),
  openItem: (id, fileIndex) => invoke('util:open', id, fileIndex),
  copyToClipboard: text => invoke('util:clipboard', text),
  contextMenu: (kind, payload) => invoke('util:contextMenu', kind, payload),
  saveTorrentFile: (id, dest) => invoke('util:saveTorrent', id, dest),
  setBadge: text => invoke('util:badge', text),
  setProgress: value => invoke('util:progress', value),

  /** Resolves a dropped File object to an absolute path (Electron 32+ API). */
  pathForFile: file => {
    try { return webUtils.getPathForFile(file) } catch { return null }
  },

  // ---- events pushed from main
  on: (channel, handler) => {
    const allowed = ['tick', 'log', 'changed', 'menu', 'open-torrent', 'settings-changed', 'update']
    if (!allowed.includes(channel)) return () => {}
    const listener = (_e, payload) => handler(payload)
    ipcRenderer.on(channel, listener)
    return () => ipcRenderer.removeListener(channel, listener)
  }
})
