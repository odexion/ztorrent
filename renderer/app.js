import { icon } from './icons.js'
import * as fmt from './util.js'
import { openAddDialog, openUrlDialog, openCreateDialog, openPreferences, openPrompt, openProperties } from './dialogs.js'

const api = window.ztorrent
const $ = sel => document.querySelector(sel)

/** Canvas can't read CSS variables, so resolve them off the root element. */
const cssVar = n => getComputedStyle(document.documentElement).getPropertyValue(n).trim()

/* ══════════════════════════════════════════════════════════════ column model */

const ALL_COLUMNS = [
  { key: '#',             label: '#',            w: 44,  num: true,  sort: r => r.order },
  { key: 'name',          label: 'Name',         w: 360, sort: r => r.name, cmp: fmt.compareText },
  { key: 'size',          label: 'Size',         w: 78,  num: true,  sort: r => r.size },
  { key: 'status',        label: 'Status',       w: 220,             sort: r => r.done },
  { key: 'seeds',         label: 'Seeds',        w: 68,  num: true,  sort: r => r.seeds },
  { key: 'peers',         label: 'Peers',        w: 68,  num: true,  sort: r => r.peers },
  { key: 'downloadSpeed', label: 'Down Speed',   w: 110,  num: true,  sort: r => r.downloadSpeed },
  { key: 'uploadSpeed',   label: 'Up Speed',     w: 88,  num: true,  sort: r => r.uploadSpeed },
  { key: 'eta',           label: 'ETA',          w: 72,  num: true,  sort: r => r.eta },
  { key: 'downloaded',    label: 'Downloaded',   w: 86,  num: true,  sort: r => r.downloaded },
  { key: 'uploaded',      label: 'Uploaded',     w: 86,  num: true,  sort: r => r.uploaded },
  { key: 'ratio',         label: 'Ratio',        w: 68,  num: true,  sort: r => r.ratio },
  { key: 'availability',  label: 'Avail.',       w: 68,  num: true,  sort: r => r.availability },
  { key: 'label',         label: 'Label',        w: 88,              sort: r => r.label, cmp: fmt.compareText },
  { key: 'addedOn',       label: 'Added On',     w: 116,             sort: r => r.addedOn },
  { key: 'completedOn',   label: 'Completed On', w: 116,             sort: r => r.completedOn },
  { key: 'savePath',      label: 'Save Path',    w: 200,             sort: r => r.savePath, cmp: fmt.compareText }
]

const DEFAULT_VISIBLE = ['#', 'name', 'size', 'status', 'seeds', 'peers',
  'downloadSpeed', 'uploadSpeed', 'eta', 'ratio', 'addedOn']

/* ═══════════════════════════════════════════════════════════════════ ui state */

const S = {
  rows: [],
  globals: {},
  details: null,
  settings: {},
  labels: [],
  selection: new Set(),
  anchor: null,
  category: 'all',
  search: '',
  sortKey: '#',
  sortDir: 1,
  tab: 'general',
  columns: DEFAULT_VISIBLE.slice(),
  widths: {},
  log: [],
  history: new Map(),        // torrent id -> {down:[], up:[]}
  globalHistory: { down: [], up: [] },
  showSidebar: true,
  showDetail: true,
  showStatusbar: true,
  didAutoSelect: false,
  dragIds: null            // torrent ids being dragged onto a label, if any
}
const HISTORY_LEN = 150

/* ═══════════════════════════════════════════════════════════════ status text */

const STATUS_LABEL = {
  downloading: 'Downloading',
  seeding: 'Seeding',
  paused: 'Paused',
  stopped: 'Stopped',
  queued: 'Queued',
  checking: 'Checking',
  metadata: 'Downloading metadata',
  finished: 'Finished',
  error: 'Error'
}

function statusText (r) {
  if (r.state === 'error') return `Error: ${r.error || 'unknown'}`
  if (r.state === 'checking') return `Checking ${fmt.pct(r.done, 1)}`
  if (r.state === 'stopped' && r.done >= 1) return 'Finished'
  return STATUS_LABEL[r.state] || r.state
}

const isActive = r =>
  (r.state === 'downloading' || r.state === 'seeding' || r.state === 'metadata') &&
  (r.downloadSpeed > 0 || r.uploadSpeed > 0 || r.numPeers > 0)

/* ═══════════════════════════════════════════════════════════════════ boot */

init()

async function init () {
  S.settings = await api.getSettings()
  S.labels = await api.getLabels()
  S.log = await api.getLog()
  const savedCols = await api.getColumns()
  // 'done' merged into 'status'; drop it from orders saved by older builds.
  if (savedCols?.order?.length) S.columns = savedCols.order.filter(k => k !== 'done')
  if (savedCols?.widths) S.widths = savedCols.widths
  applyTheme()

  buildToolbar()
  buildTabstrip()
  renderSidebar()
  renderHead()
  renderStatusbar()
  wireEvents()

  const snap = await api.getSnapshot()
  S.rows = snap.rows
  S.globals = snap.globals
  renderAll()

  api.on('tick', onTick)
  api.on('log', line => {
    S.log.push(line)
    if (S.log.length > 800) S.log.shift()
    if (S.tab === 'logger') renderLogger()
  })
  api.on('changed', async () => {
    S.labels = await api.getLabels()
    renderSidebar()
  })
  api.on('settings-changed', s => { S.settings = s; applyTheme(); renderStatusbar() })
  api.on('menu', onMenu)
  api.on('open-torrent', src => openAddDialog(src))
}

function applyTheme () {
  document.documentElement.dataset.theme = S.settings.theme || 'classic'
}

/* ═══════════════════════════════════════════════════════════════════ ticking */

function onTick ({ rows, globals, details }) {
  S.rows = rows
  S.globals = globals
  S.details = details

  pushHistory(S.globalHistory, globals.downloadSpeed, globals.uploadSpeed)
  for (const r of rows) {
    if (!S.history.has(r.id)) S.history.set(r.id, { down: [], up: [] })
    pushHistory(S.history.get(r.id), r.downloadSpeed, r.uploadSpeed)
  }
  for (const id of [...S.history.keys()]) {
    if (!rows.some(r => r.id === id)) S.history.delete(id)
  }

  // Drop selections whose torrents have gone away.
  let changed = false
  for (const id of [...S.selection]) {
    if (!rows.some(r => r.id === id)) { S.selection.delete(id); changed = true }
  }
  if (changed) syncSelection()

  // Give the detail pane something to show the first time torrents appear.
  if (!S.didAutoSelect && rows.length) {
    S.didAutoSelect = true
    if (!S.selection.size) selectId(filteredRows()[0]?.id ?? rows[0].id)
  }

  renderAll()
}

function pushHistory (h, down, up) {
  h.down.push(down); h.up.push(up)
  if (h.down.length > HISTORY_LEN) { h.down.shift(); h.up.shift() }
}

function renderAll () {
  renderSidebarCounts()
  renderList()
  renderStatusbar()
  renderToolbarState()
  renderDetail()
}

/* ═══════════════════════════════════════════════════════════════════ toolbar */

/* Shortcut hints are written the way each platform writes them. */
const IS_MAC = api.platform === 'darwin'
const accel = (k, shift) => IS_MAC
  ? `${shift ? '⇧' : ''}⌘${k}`
  : `Ctrl+${shift ? 'Shift+' : ''}${k}`
const DELETE_KEY = IS_MAC ? '⌘⌫' : 'Del'

const TOOLBAR_HINTS = {
  'add-file': ['Add Torrent', accel('O')],
  'add-url': ['Add Torrent from URL', accel('U')],
  create: ['Create New Torrent', accel('N')],
  remove: ['Remove', DELETE_KEY],
  start: ['Start', accel('R')],
  pause: ['Pause', accel('P')],
  stop: ['Stop', accel('.')],
  'queue-up': ['Move Up Queue', IS_MAC ? '⌘↑' : 'Ctrl+Up'],
  'queue-down': ['Move Down Queue', IS_MAC ? '⌘↓' : 'Ctrl+Down'],
  'alt-speed': ['Alternate Speed Limits', accel('L', true)],
  preferences: ['Preferences', accel(',')]
}

function buildToolbar () {
  for (const btn of document.querySelectorAll('#toolbar .tb-btn')) {
    btn.innerHTML = icon(btn.dataset.act)
    const hint = TOOLBAR_HINTS[btn.dataset.act]
    if (hint) btn.title = `${hint[0]}  (${hint[1]})`
  }
  $('#search-icon').innerHTML = icon('search')
}

const TOOLBAR_NEEDS_SELECTION = new Set(['remove', 'start', 'pause', 'stop', 'queue-up', 'queue-down'])

function renderToolbarState () {
  const n = S.selection.size
  for (const btn of document.querySelectorAll('#toolbar .tb-btn')) {
    const act = btn.dataset.act
    if (TOOLBAR_NEEDS_SELECTION.has(act)) {
      const need = (act === 'queue-up' || act === 'queue-down') ? n === 1 : n > 0
      btn.classList.toggle('disabled', !need)
    }
    if (act === 'alt-speed') btn.classList.toggle('active', !!S.settings.altSpeedEnabled)
  }
}

/* ═══════════════════════════════════════════════════════════════════ sidebar */

function categoryFilter (cat) {
  switch (cat) {
    case 'all':         return () => true
    case 'downloading': return r => r.state === 'downloading' || r.state === 'metadata' ||
                                    (r.done < 1 && (r.state === 'queued' || r.state === 'checking'))
    case 'seeding':     return r => r.state === 'seeding'
    case 'completed':   return r => r.done >= 1
    case 'active':      return isActive
    case 'inactive':    return r => !isActive(r)
    case 'nolabel':     return r => !r.label
    default:
      if (cat.startsWith('label:')) { const l = cat.slice(6); return r => r.label === l }
      return () => true
  }
}

const SIDEBAR_ITEMS = [
  { id: 'downloading', label: 'Downloading', ic: 'downloading' },
  { id: 'seeding',     label: 'Seeding',     ic: 'seeding' },
  { id: 'completed',   label: 'Completed',   ic: 'completed' },
  { id: 'active',      label: 'Active',      ic: 'active' },
  { id: 'inactive',    label: 'Inactive',    ic: 'inactive' }
]

function renderSidebar () {
  const item = (id, label, ic, cls = '') => `
    <div class="tree-item ${cls} ${S.category === id ? 'selected' : ''}" data-cat="${id}">
      ${ic ? icon(ic) : '<span style="width:13px"></span>'}
      <span class="lbl">${fmt.esc(label)}</span>
      <span class="cnt" data-count="${id}"></span>
    </div>`

  let html = `<div class="tree-group">
    ${item('all', 'Torrents', 'torrents', 'root')}
    <div class="tree-children">
      ${SIDEBAR_ITEMS.map(i => item(i.id, i.label, i.ic, 'child')).join('')}
    </div>
  </div>`

  html += `<div class="tree-group">
    <div class="group-title">Labels</div>
    <div class="tree-children">
      ${item('nolabel', 'No Label', 'label', 'child')}
      ${S.labels.map(l => item('label:' + l, l, 'label', 'child')).join('')}
      <div class="tree-item child new-label" data-drop="new">
        ${icon('create')}<span class="lbl">New Label…</span>
      </div>
    </div>
  </div>`

  $('#sidebar').innerHTML = html
  renderSidebarCounts()
}

function renderSidebarCounts () {
  for (const el of document.querySelectorAll('#sidebar [data-count]')) {
    const cat = el.dataset.count
    el.textContent = `(${S.rows.filter(categoryFilter(cat)).length})`
  }
}

/* ═════════════════════════════════════════════════════════════ torrent list */

function visibleColumns () {
  return S.columns.map(k => ALL_COLUMNS.find(c => c.key === k)).filter(Boolean)
}

function colWidth (c) { return S.widths[c.key] ?? c.w }

function colStyle (c) {
  const w = colWidth(c)
  return `flex:0 0 ${w}px;width:${w}px`
}

function renderHead () {
  const head = $('#list-head')
  head.innerHTML = visibleColumns().map(c => {
    const arrow = S.sortKey === c.key ? (S.sortDir > 0 ? '▲' : '▼') : ''
    return `<div class="gh ${c.num ? 'num' : ''}" data-key="${c.key}" style="${colStyle(c)}">
      <span class="lbl">${fmt.esc(c.label)}</span><span class="sort">${arrow}</span>
      <span class="grip" data-grip="${c.key}"></span>
    </div>`
  }).join('')
}

function filteredRows () {
  const catFn = categoryFilter(S.category)
  const q = S.search.trim().toLowerCase()
  let rows = S.rows.filter(r => catFn(r) && (!q || r.name.toLowerCase().includes(q) ||
    r.label.toLowerCase().includes(q)))

  const col = ALL_COLUMNS.find(c => c.key === S.sortKey) || ALL_COLUMNS[0]
  const cmp = col.cmp || ((a, b) => (a === b ? 0 : a < b ? -1 : 1))
  rows = rows.slice().sort((a, b) => S.sortDir * cmp(col.sort(a), col.sort(b)))
  return rows
}

const rowEls = new Map()

function renderList () {
  const body = $('#list-body')
  const rows = filteredRows()

  if (!rows.length) {
    if (!body.querySelector('#empty-hint')) {
      rowEls.clear()
      body.innerHTML = `<div id="empty-hint">
        ${icon('logo')}
        <div>${S.rows.length ? 'No torrents match this view.' : 'No torrents yet.'}</div>
        <div>Drop a <b>.torrent</b> file here, or press <kbd>${accel('O')}</kbd> to add one.</div>
      </div>`
    } else {
      const msg = body.querySelector('#empty-hint div')
      const want = S.rows.length ? 'No torrents match this view.' : 'No torrents yet.'
      if (msg.textContent !== want) msg.textContent = want
    }
    return
  }
  const hint = body.querySelector('#empty-hint')
  if (hint) { hint.remove(); rowEls.clear() }

  const cols = visibleColumns()
  const seen = new Set()

  rows.forEach((r, i) => {
    seen.add(r.id)
    let entry = rowEls.get(r.id)
    if (!entry || entry.sig !== cols.map(c => c.key).join(',')) {
      const el = document.createElement('div')
      el.className = 'grid-row'
      el.draggable = true
      el.dataset.id = r.id
      entry = { el, cells: new Map(), sig: cols.map(c => c.key).join(','), cache: new Map() }
      for (const c of cols) {
        const cell = document.createElement('div')
        cell.className = `cell ${c.key} ${c.num ? 'num' : ''}`
        cell.style.cssText = colStyle(c)
        el.appendChild(cell)
        entry.cells.set(c.key, cell)
      }
      rowEls.set(r.id, entry)
    }
    for (const c of cols) {
      const cell = entry.cells.get(c.key)
      cell.style.cssText = colStyle(c)
      paintCell(cell, c.key, r, i, entry.cache)
    }
    entry.el.classList.toggle('selected', S.selection.has(r.id))
  })

  for (const [id, entry] of rowEls) {
    if (!seen.has(id)) { entry.el.remove(); rowEls.delete(id) }
  }

  // Reorder in place, touching the DOM only where the order actually differs.
  let node = body.firstElementChild
  for (const r of rows) {
    const el = rowEls.get(r.id).el
    if (node !== el) { body.insertBefore(el, node) } else { node = node.nextElementSibling }
  }
}

function paintCell (cell, key, r, index, cache) {
  let html = null
  let text = null

  switch (key) {
    case '#': text = String(index + 1); break
    case 'name':
      const st = stateIcon(r)
      html = `${icon(st, null, 'st-' + st)}<span>${fmt.esc(r.name)}</span>`
      cell.title = r.name
      break
    case 'size': text = fmt.bytes(r.wantedSize || r.size); break
    case 'status': {
      const w = (Math.max(0, Math.min(1, r.done)) * 100).toFixed(2)
      const cls = barClass(r)
      // Progress reads from the bar, so only show the number while it's moving.
      const label = (r.done >= 1 || r.state === 'error')
        ? statusText(r)
        : `${statusText(r)} ${fmt.pct(r.done, 1)}`
      const t = fmt.esc(label)
      html = `<div class="pbar ${cls}" style="--p:${w}%"><i style="width:${w}%"></i>` +
             `<b>${t}</b><b class="on">${t}</b></div>`
      break
    }
    case 'seeds': text = r.state === 'stopped' ? '' : String(r.seeds); break
    case 'peers': text = r.state === 'stopped' ? '' : String(r.peers); break
    case 'downloadSpeed': text = fmt.speed(r.downloadSpeed); break
    case 'uploadSpeed': text = fmt.speed(r.uploadSpeed); break
    case 'eta': text = (r.done >= 1 || r.downloadSpeed === 0) ? '' : fmt.eta(r.eta); break
    case 'downloaded': text = fmt.bytes(r.downloaded, true); break
    case 'uploaded': text = fmt.bytes(r.uploaded, true); break
    case 'ratio': text = fmt.ratio(r.ratio); break
    case 'availability': text = r.availability ? r.availability.toFixed(3) : ''; break
    case 'label': text = r.label; break
    case 'addedOn': text = fmt.datetime(r.addedOn); break
    case 'completedOn': text = fmt.datetime(r.completedOn); break
    case 'savePath': text = r.savePath; cell.title = r.savePath; break
  }

  const val = html ?? text
  if (cache.get(key) === val) return
  cache.set(key, val)
  if (html !== null) cell.innerHTML = html
  else cell.textContent = text
}

/* Complete torrents split two ways: green while they're still giving back to
   the swarm, purple once they've stopped -- 'Finished' is its own state, not a
   quieter kind of seeding. */
function barClass (r) {
  if (r.state === 'error') return 'err'
  if (r.done >= 1) return r.state === 'seeding' ? 'seed' : 'fin'
  if (r.state === 'downloading' || r.state === 'checking' || r.state === 'metadata') return ''
  return 'idle'
}

function stateIcon (r) {
  if (r.state === 'error') return 'error'
  if (r.state === 'seeding') return 'seeding'
  if (r.state === 'downloading' || r.state === 'metadata' || r.state === 'checking') return 'downloading'
  if (r.done >= 1) return 'completed'
  return 'inactive'
}

/* ═══════════════════════════════════════════════════════════════ status bar */

/** Mirrors µTorrent's status-bar limit readout: whichever caps are in force. */
function speedCapLabel () {
  const s = S.settings
  const alt = s.altSpeedEnabled
  const dn = alt ? s.altDownloadRate : s.maxDownloadRate
  const up = alt ? s.altUploadRate : s.maxUploadRate
  if (!dn && !up) return 'No limit'
  const part = (v, sym) => `${sym}${v > 0 ? v + ' kB/s' : '∞'}`
  return `${alt ? 'Alt' : 'Limit'}: ${part(dn, '↓')} ${part(up, '↑')}`
}

function renderStatusbar () {
  const g = S.globals
  const dhtOk = g.dhtReady
  const dhtEl = $('#sb-dht')
  dhtEl.innerHTML = S.settings.enableDHT
    ? `<span class="dot ${dhtOk ? 'green' : 'yellow'}"></span><span>DHT: ${dhtOk ? `${g.dhtNodes} nodes` : 'starting'}</span>`
    : `<span class="dot red"></span><span>DHT: disabled</span>`

  $('#sb-count').innerHTML =
    `<span class="k">Torrents:</span><span>${S.rows.length}</span>`

  $('#sb-alt').innerHTML = `${icon('alt-speed')}<span>${speedCapLabel()}</span>`

  $('#sb-down').innerHTML =
    `<span style="color:var(--down)">${icon('down')}</span>` +
    `<span>D: ${fmt.speed(g.downloadSpeed, false)}</span>` +
    `<span class="k">T: ${fmt.bytes(g.downloaded)}</span>`

  $('#sb-up').innerHTML =
    `<span style="color:var(--up)">${icon('up')}</span>` +
    `<span>U: ${fmt.speed(g.uploadSpeed, false)}</span>` +
    `<span class="k">T: ${fmt.bytes(g.uploaded)}</span>`
}

/* ═════════════════════════════════════════════════════════════ detail tabs */

const TABS = [
  { id: 'general',  label: 'General',  ic: 'info' },
  { id: 'trackers', label: 'Trackers', ic: 'tracker' },
  { id: 'peers',    label: 'Peers',    ic: 'peers' },
  { id: 'pieces',   label: 'Pieces',   ic: 'pieces' },
  { id: 'files',    label: 'Files',    ic: 'files' },
  { id: 'speed',    label: 'Speed',    ic: 'speed' },
  { id: 'logger',   label: 'Logger',   ic: 'logger' }
]

function buildTabstrip () {
  $('#tabstrip').innerHTML = TABS.map(t =>
    `<div class="tab ${t.id === S.tab ? 'active' : ''}" data-tab="${t.id}">
       ${icon(t.ic)}<span>${t.label}</span>
     </div>`).join('')
}

function setTab (id) {
  S.tab = id
  for (const el of document.querySelectorAll('.tab')) el.classList.toggle('active', el.dataset.tab === id)
  for (const el of document.querySelectorAll('.tabpane')) el.classList.toggle('active', el.id === 'pane-' + id)
  renderDetail()
}

const selectedRow = () => S.rows.find(r => S.selection.has(r.id) && r.id === [...S.selection][0]) ||
                          S.rows.find(r => S.selection.has(r.id))

function renderDetail () {
  if (!S.showDetail) return
  switch (S.tab) {
    case 'general':  renderGeneral(); break
    case 'trackers': renderTrackers(); break
    case 'peers':    renderPeers(); break
    case 'pieces':   renderPieces(); break
    case 'files':    renderFiles(); break
    case 'speed':    renderSpeed(); break
    case 'logger':   renderLogger(); break
  }
}

function emptyPane (el, msg = 'Select a torrent to see its details.') {
  el.innerHTML = `<div style="padding:18px;color:var(--ink-faint)">${msg}</div>`
}

function renderGeneral () {
  const el = $('#pane-general')
  const r = selectedRow()
  if (!r) return emptyPane(el)
  const d = S.details && S.details.id === r.id ? S.details : null
  const elapsed = Date.now() - r.addedOn
  const remaining = r.done >= 1 ? 0 : r.eta

  el.innerHTML = `
    <div class="gen-bar">
      <div class="pbar ${barClass(r)}">
        <i style="width:${(r.done * 100).toFixed(2)}%"></i>
      </div>
      <span class="pct">${fmt.pct(r.done, 2)}</span>
    </div>
    <div class="gen-cols">
      <div class="gen-block">
        <div class="gen-title">Transfer</div>
        <dl class="gen-grid">
          <dt>Time Elapsed:</dt><dd>${fmt.duration(elapsed)}</dd>
          <dt>Remaining:</dt><dd>${r.done >= 1 ? '—' : fmt.eta(remaining)}</dd>
          <dt>Downloaded:</dt><dd>${fmt.bytes(r.downloaded)}</dd>
          <dt>Uploaded:</dt><dd>${fmt.bytes(r.uploaded)}</dd>
          <dt>Download Speed:</dt><dd>${fmt.speed(r.downloadSpeed, false)}</dd>
          <dt>Upload Speed:</dt><dd>${fmt.speed(r.uploadSpeed, false)}</dd>
          <dt>Share Ratio:</dt><dd>${fmt.ratio(r.ratio)}</dd>
          <dt>Seeds:</dt><dd>${r.seeds} connected</dd>
          <dt>Peers:</dt><dd>${r.peers} connected</dd>
          <dt>Availability:</dt><dd>${r.availability ? r.availability.toFixed(3) : '—'}</dd>
          <dt>Status:</dt><dd>${fmt.esc(statusText(r))}</dd>
        </dl>
      </div>
      <div class="gen-block">
        <div class="gen-title">Torrent</div>
        <dl class="gen-grid">
          <dt>Name:</dt><dd title="${fmt.esc(r.name)}">${fmt.esc(r.name)}</dd>
          <dt>Save As:</dt><dd title="${fmt.esc(r.savePath)}">${fmt.esc(r.savePath)}</dd>
          <dt>Total Size:</dt><dd>${fmt.bytes(r.size)}${r.wantedSize !== r.size ? ` (${fmt.bytes(r.wantedSize)} selected)` : ''}</dd>
          <dt>Pieces:</dt><dd>${d ? `${d.pieceCount} × ${fmt.bytes(d.pieceLength)} (have ${d.have})` : '—'}</dd>
          <dt>Hash:</dt><dd class="sel" title="${fmt.esc(r.infoHash)}">${fmt.esc(r.infoHash)}</dd>
          <dt>Comment:</dt><dd title="${fmt.esc(d?.comment)}">${fmt.esc(d?.comment) || '—'}</dd>
          <dt>Created By:</dt><dd>${fmt.esc(d?.createdBy) || '—'}</dd>
          <dt>Created On:</dt><dd>${d?.createdOn ? fmt.datetime(d.createdOn) : '—'}</dd>
          <dt>Added On:</dt><dd>${fmt.datetime(r.addedOn)}</dd>
          <dt>Completed On:</dt><dd>${r.completedOn ? fmt.datetime(r.completedOn) : '—'}</dd>
          <dt>Private:</dt><dd>${d ? (d.private ? 'Yes (DHT/PEX off)' : 'No') : '—'}</dd>
          <dt>Order:</dt><dd>${d?.sequential ? 'Sequential' : 'Rarest first'}</dd>
        </dl>
      </div>
    </div>`
}

function renderTrackers () {
  const el = $('#pane-trackers')
  const d = S.details
  if (!d) return emptyPane(el)
  const rows = d.trackers.map(t => `
    <tr data-url="${fmt.esc(t.url)}">
      <td title="${fmt.esc(t.url)}">${fmt.esc(t.url)}</td>
      <td class="${t.status === 'Working' ? 'ok' : t.status === 'Not working' ? 'bad' : 'dim'}">${fmt.esc(t.status)}</td>
      <td class="num">${t.seeds >= 0 ? t.seeds : '—'}</td>
      <td class="num">${t.peers >= 0 ? t.peers : '—'}</td>
      <td class="num">${t.interval ? t.interval + 's' : '—'}</td>
    </tr>`).join('')
  el.innerHTML = `<table class="dtable" id="tracker-table">
    <thead><tr><th style="width:52%">Tracker</th><th style="width:18%">Status</th>
      <th class="num">Seeds</th><th class="num">Peers</th><th class="num">Update In</th></tr></thead>
    <tbody>${rows || '<tr><td colspan="5" class="dim">No trackers.</td></tr>'}</tbody></table>`
}

function renderPeers () {
  const el = $('#pane-peers')
  const d = S.details
  if (!d) return emptyPane(el)
  const rows = d.peers
    .sort((a, b) => (b.downSpeed + b.upSpeed) - (a.downSpeed + a.upSpeed))
    .map(p => `
    <tr data-addr="${fmt.esc(p.address)}:${p.port}">
      <td title="${fmt.esc(p.address)}:${p.port}">${fmt.esc(p.address)}</td>
      <td class="dim">${fmt.esc(p.type)}</td>
      <td class="flags">${fmt.esc(p.flags)}</td>
      <td title="${fmt.esc(p.client)}">${fmt.esc(p.client)}</td>
      <td class="num">${fmt.pct(p.progress, 1)}</td>
      <td class="num">${fmt.speed(p.downSpeed)}</td>
      <td class="num">${fmt.speed(p.upSpeed)}</td>
      <td class="num">${fmt.bytes(p.downloaded, true)}</td>
      <td class="num">${fmt.bytes(p.uploaded, true)}</td>
    </tr>`).join('')
  el.innerHTML = `<table class="dtable" id="peer-table">
    <thead><tr><th style="width:16%">IP</th><th style="width:74px">Conn</th><th style="width:58px">Flags</th>
      <th style="width:22%">Client</th><th class="num" style="width:58px">%</th>
      <th class="num" style="width:94px">Down Speed</th><th class="num" style="width:88px">Up Speed</th>
      <th class="num" style="width:96px">Downloaded</th><th class="num" style="width:96px">Uploaded</th></tr></thead>
    <tbody>${rows || '<tr><td colspan="9" class="dim">No peers connected.</td></tr>'}</tbody></table>`
}

let piecesCanvas = null
function renderPieces () {
  const el = $('#pane-pieces')
  const d = S.details
  if (!d) return emptyPane(el)

  if (!el.querySelector('canvas')) {
    el.innerHTML = `<div class="canvas-pane">
      <div class="cv-head">
        <span class="legend"><i style="background:var(--bar-seed)"></i>Have</span>
        <span class="legend"><i style="background:var(--piece-partial)"></i>Downloading</span>
        <span class="legend"><i style="background:var(--bar-track)"></i>Missing</span>
        <span id="pieces-stat"></span>
      </div>
      <canvas id="pieces-canvas"></canvas>
    </div>`
    piecesCanvas = el.querySelector('#pieces-canvas')
  }
  const stat = el.querySelector('#pieces-stat')
  if (!d.pieces) { stat.textContent = 'Waiting for metadata…'; return }

  const map = base64Bytes(d.pieces)
  stat.innerHTML = `<b>${d.have}</b> of <b>${map.length}</b> pieces · ${fmt.bytes(d.pieceLength)} each`
  drawPieces(piecesCanvas, map)
}

function base64Bytes (b64) {
  const bin = atob(b64)
  const out = new Uint8Array(bin.length)
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i)
  return out
}

function drawPieces (canvas, map) {
  const dpr = window.devicePixelRatio || 1
  const w = canvas.clientWidth
  const h = canvas.clientHeight
  if (!w || !h) return
  canvas.width = w * dpr
  canvas.height = h * dpr
  const ctx = canvas.getContext('2d')
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0)

  ctx.fillStyle = cssVar('--bg')
  ctx.fillRect(0, 0, w, h)

  const n = map.length
  if (!n) return
  const pad = 6
  const availW = w - pad * 2
  const availH = h - pad * 2

  // Choose a cell size that fits every piece into the available box.
  let cell = Math.floor(Math.sqrt((availW * availH) / n))
  cell = Math.max(2, Math.min(14, cell))
  let perRow = Math.max(1, Math.floor(availW / cell))
  while (Math.ceil(n / perRow) * cell > availH && cell > 2) {
    cell--
    perRow = Math.max(1, Math.floor(availW / cell))
  }
  const gap = cell > 4 ? 1 : 0
  const COLORS = [cssVar('--bar-track'), cssVar('--piece-partial'), cssVar('--bar-seed')]

  for (let i = 0; i < n; i++) {
    const x = pad + (i % perRow) * cell
    const y = pad + Math.floor(i / perRow) * cell
    if (y > h) break
    ctx.fillStyle = COLORS[map[i]] || COLORS[0]
    ctx.fillRect(x, y, cell - gap, cell - gap)
  }
}

function renderFiles () {
  const el = $('#pane-files')
  const d = S.details
  if (!d) return emptyPane(el)
  if (!d.files.length || d.files[0].name === undefined) {
    return emptyPane(el, 'File list appears once the torrent metadata arrives.')
  }
  const PRIO = { 0: "Don't Download", 1: 'Normal', 2: 'High' }
  const rows = d.files.map(f => `
    <tr data-index="${f.index}" data-priority="${f.priority}">
      <td class="num dim">${f.index + 1}</td>
      <td title="${fmt.esc(f.path)}">${fmt.esc(f.name)}</td>
      <td class="num">${fmt.bytes(f.length)}</td>
      <td><div class="minibar"><i style="width:${(f.progress * 100).toFixed(1)}%"></i></div></td>
      <td class="num">${fmt.pct(f.progress, 1)}</td>
      <td class="prio-${f.priority}">${PRIO[f.priority]}</td>
    </tr>`).join('')
  el.innerHTML = `<table class="dtable" id="file-table">
    <thead><tr><th class="num" style="width:36px">#</th><th style="width:46%">Name</th>
      <th class="num" style="width:84px">Size</th><th style="width:90px">Progress</th>
      <th class="num" style="width:56px">%</th><th style="width:110px">Priority</th></tr></thead>
    <tbody>${rows}</tbody></table>`
}

let speedCanvas = null
function renderSpeed () {
  const el = $('#pane-speed')
  const r = selectedRow()
  const h = r ? (S.history.get(r.id) || { down: [], up: [] }) : S.globalHistory
  const title = r ? fmt.esc(r.name) : 'All torrents'

  if (!el.querySelector('canvas')) {
    el.innerHTML = `<div class="canvas-pane">
      <div class="cv-head">
        <span class="legend"><i style="background:var(--down)"></i>Download</span>
        <span class="legend"><i style="background:var(--up)"></i>Upload</span>
        <span id="speed-scope"></span>
        <span id="speed-stat"></span>
      </div>
      <canvas id="speed-canvas"></canvas>
    </div>`
    speedCanvas = el.querySelector('#speed-canvas')
  }
  el.querySelector('#speed-scope').innerHTML = `Showing: <b>${title}</b>`
  const dNow = h.down.at(-1) || 0
  const uNow = h.up.at(-1) || 0
  el.querySelector('#speed-stat').innerHTML =
    `D <b>${fmt.speed(dNow, false)}</b> · U <b>${fmt.speed(uNow, false)}</b>`
  drawSpeed(speedCanvas, h)
}

function drawSpeed (canvas, h) {
  const dpr = window.devicePixelRatio || 1
  const w = canvas.clientWidth
  const ht = canvas.clientHeight
  if (!w || !ht) return
  canvas.width = w * dpr
  canvas.height = ht * dpr
  const ctx = canvas.getContext('2d')
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0)

  ctx.fillStyle = cssVar('--bg')
  ctx.fillRect(0, 0, w, ht)

  const padL = 58; const padR = 8; const padT = 8; const padB = 16
  const gw = w - padL - padR
  const gh = ht - padT - padB
  if (gw <= 0 || gh <= 0) return

  const peak = Math.max(1024, ...h.down, ...h.up) * 1.15

  // Grid + y labels
  ctx.strokeStyle = cssVar('--line')
  ctx.fillStyle = cssVar('--ink-faint')
  ctx.font = '11px -apple-system, system-ui, sans-serif'
  ctx.textAlign = 'right'
  ctx.textBaseline = 'middle'
  ctx.lineWidth = 1
  for (let i = 0; i <= 4; i++) {
    const y = Math.round(padT + (gh * i) / 4) + 0.5
    ctx.beginPath(); ctx.moveTo(padL, y); ctx.lineTo(padL + gw, y); ctx.stroke()
    ctx.fillText(fmt.speed(peak * (1 - i / 4), false), padL - 6, y)
  }

  const n = HISTORY_LEN
  const xAt = i => padL + (gw * i) / (n - 1)
  const yAt = v => padT + gh - (Math.min(v, peak) / peak) * gh

  const series = (data, stroke, fill) => {
    if (!data.length) return
    const off = n - data.length
    ctx.beginPath()
    ctx.moveTo(xAt(off), padT + gh)
    data.forEach((v, i) => ctx.lineTo(xAt(off + i), yAt(v)))
    ctx.lineTo(xAt(off + data.length - 1), padT + gh)
    ctx.closePath()
    ctx.fillStyle = fill
    ctx.fill()

    ctx.beginPath()
    data.forEach((v, i) => (i ? ctx.lineTo(xAt(off + i), yAt(v)) : ctx.moveTo(xAt(off), yAt(v))))
    ctx.strokeStyle = stroke
    ctx.lineWidth = 1.4
    ctx.stroke()
  }

  series(h.down, cssVar('--down'), cssVar('--chart-down-fill'))
  series(h.up, cssVar('--up'), cssVar('--chart-up-fill'))

  ctx.strokeStyle = cssVar('--line-hard')
  ctx.beginPath()
  ctx.moveTo(padL + 0.5, padT); ctx.lineTo(padL + 0.5, padT + gh + 0.5); ctx.lineTo(padL + gw, padT + gh + 0.5)
  ctx.stroke()

  ctx.textAlign = 'center'
  ctx.fillStyle = cssVar('--ink-faint')
  ctx.fillText(`${n}s ago`, padL + 26, ht - 6)
  ctx.fillText('now', padL + gw - 12, ht - 6)
}

function renderLogger () {
  const el = $('#pane-logger')
  const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 24
  el.innerHTML = S.log.map(l =>
    `<div class="logline ${l.level}"><span class="t">[${fmt.clock(l.time)}]</span> ${fmt.esc(l.message)}</div>`
  ).join('')
  if (atBottom) el.scrollTop = el.scrollHeight
}

/* ══════════════════════════════════════════════════════════════════ selection */

function syncSelection () {
  for (const [id, entry] of rowEls) entry.el.classList.toggle('selected', S.selection.has(id))
  const first = [...S.selection][0] || null
  api.getDetails(first).then(d => { S.details = d; renderDetail() })
  renderToolbarState()
}

function selectId (id, { additive = false, range = false } = {}) {
  const rows = filteredRows()
  if (range && S.anchor) {
    const a = rows.findIndex(r => r.id === S.anchor)
    const b = rows.findIndex(r => r.id === id)
    if (a >= 0 && b >= 0) {
      if (!additive) S.selection.clear()
      for (let i = Math.min(a, b); i <= Math.max(a, b); i++) S.selection.add(rows[i].id)
    }
  } else if (additive) {
    if (S.selection.has(id)) S.selection.delete(id)
    else S.selection.add(id)
    S.anchor = id
  } else {
    S.selection.clear()
    S.selection.add(id)
    S.anchor = id
  }
  syncSelection()
}

const ids = () => [...S.selection]

/* ═══════════════════════════════════════════════════════════════════ actions */

async function doAction (act, arg) {
  const sel = ids()
  switch (act) {
    case 'add-file': {
      const paths = await api.addTorrentDialog()
      if (paths.length === 1) openAddDialog(paths[0])
      else if (paths.length > 1) await api.addTorrentPaths(paths)
      break
    }
    case 'add-url': openUrlDialog(); break
    case 'create': openCreateDialog(); break
    case 'preferences': openPreferences(S.settings, patch => api.setSettings(patch)); break
    case 'start': if (sel.length) await api.start(sel); break
    case 'pause': if (sel.length) await api.pause(sel); break
    case 'stop': if (sel.length) await api.stop(sel); break
    case 'remove': if (sel.length) await api.remove(sel, false); break
    case 'remove-data': if (sel.length) await api.remove(sel, true); break
    case 'recheck': if (sel.length) await api.recheck(sel); break
    case 'reannounce': if (sel.length === 1) await api.reannounce(sel[0]); break
    case 'queue-up': if (sel.length === 1) await api.moveQueue(sel[0], -1); break
    case 'queue-down': if (sel.length === 1) await api.moveQueue(sel[0], 1); break
    case 'alt-speed': {
      const on = await api.toggleAltSpeed()
      S.settings.altSpeedEnabled = on
      renderStatusbar(); renderToolbarState()
      break
    }
    case 'reveal': if (sel.length === 1) await api.revealInFinder(sel[0]); break
    case 'magnet': {
      const r = S.rows.find(x => x.id === sel[0])
      if (r?.magnetURI) await api.copyToClipboard(r.magnetURI)
      break
    }
    case 'save-torrent': {
      const r = S.rows.find(x => x.id === sel[0])
      if (!r) break
      const dest = await api.saveFileDialog(`${r.name}.torrent`,
        [{ name: 'Torrent Files', extensions: ['torrent'] }])
      if (dest) await api.saveTorrentFile(r.id, dest)
      break
    }
    case 'properties': if (sel.length === 1) openProperties(S.rows.find(x => x.id === sel[0]), S.details); break
    case 'toggle-sequential': {
      const r = S.rows.find(x => x.id === sel[0])
      if (r) await api.setSequential(r.id, !r.sequential)
      break
    }
    case 'label-new': {
      const name = await openPrompt('New Label', 'Label name:', '')
      if (name) await api.setLabel(sel, name.trim())
      break
    }
    case 'focus-search': $('#search').focus(); $('#search').select(); break
    case 'toggle-sidebar':
      S.showSidebar = !S.showSidebar
      $('#body').classList.toggle('no-sidebar', !S.showSidebar)
      break
    case 'toggle-detail':
      S.showDetail = !S.showDetail
      $('#main').classList.toggle('no-detail', !S.showDetail)
      if (S.showDetail) renderDetail()
      break
    case 'toggle-statusbar':
      S.showStatusbar = !S.showStatusbar
      $('#statusbar').style.display = S.showStatusbar ? '' : 'none'
      break
    default:
      if (act.startsWith('label:')) await api.setLabel(sel, act.slice(6))
      if (act.startsWith('col:')) toggleColumn(act.slice(4))
  }
}

function onMenu ({ action }) { doAction(action) }

function toggleColumn (key) {
  if (key === '#' || key === 'name') return
  const i = S.columns.indexOf(key)
  if (i >= 0) S.columns.splice(i, 1)
  else {
    // Re-insert in the canonical order so the layout stays predictable.
    const order = ALL_COLUMNS.map(c => c.key)
    S.columns.push(key)
    S.columns.sort((a, b) => order.indexOf(a) - order.indexOf(b))
  }
  rowEls.clear()
  $('#list-body').innerHTML = ''
  renderHead()
  renderList()
  api.setColumns({ order: S.columns, widths: S.widths })
}

/* ════════════════════════════════════════════════════════════════════ events */

function wireEvents () {
  $('#toolbar').addEventListener('click', e => {
    const btn = e.target.closest('.tb-btn')
    if (btn && !btn.classList.contains('disabled')) doAction(btn.dataset.act)
  })

  $('#search').addEventListener('input', fmt.debounce(e => {
    S.search = e.target.value
    renderList()
  }, 120))
  $('#search').addEventListener('keydown', e => {
    if (e.key === 'Escape') { e.target.value = ''; S.search = ''; renderList(); e.target.blur() }
  })

  $('#sidebar').addEventListener('click', e => {
    if (e.target.closest('.tree-item[data-drop="new"]')) return void doAction('label-new')
    const item = e.target.closest('.tree-item[data-cat]')
    if (!item) return
    S.category = item.dataset.cat
    for (const el of document.querySelectorAll('#sidebar .tree-item')) {
      el.classList.toggle('selected', el.dataset.cat === S.category)
    }
    renderList()
  })

  // --- column header: sort, resize, chooser
  const head = $('#list-head')
  head.addEventListener('click', e => {
    if (e.target.dataset.grip) return
    const gh = e.target.closest('.gh')
    if (!gh) return
    if (S.sortKey === gh.dataset.key) S.sortDir *= -1
    else { S.sortKey = gh.dataset.key; S.sortDir = 1 }
    renderHead()
    renderList()
  })
  head.addEventListener('contextmenu', async e => {
    e.preventDefault()
    const action = await api.contextMenu('columns', {
      columns: ALL_COLUMNS.map(c => ({ key: c.key, label: c.label, on: S.columns.includes(c.key) }))
    })
    if (action) doAction(action)
  })
  head.addEventListener('mousedown', e => {
    const key = e.target.dataset.grip
    if (!key) return
    e.preventDefault()
    const col = ALL_COLUMNS.find(c => c.key === key)
    const startX = e.clientX
    const startW = colWidth(col)
    const move = ev => {
      S.widths[key] = Math.max(28, startW + ev.clientX - startX)
      renderHead()
      renderList()
    }
    const up = () => {
      window.removeEventListener('mousemove', move)
      window.removeEventListener('mouseup', up)
      api.setColumns({ order: S.columns, widths: S.widths })
    }
    window.addEventListener('mousemove', move)
    window.addEventListener('mouseup', up)
  })

  // --- list interactions
  const body = $('#list-body')
  body.addEventListener('scroll', () => { head.scrollLeft = body.scrollLeft })
  body.addEventListener('focus', () => body.classList.add('focused'))
  body.addEventListener('blur', () => body.classList.remove('focused'))
  body.classList.add('focused')

  body.addEventListener('mousedown', e => {
    const row = e.target.closest('.grid-row')
    if (!row) { S.selection.clear(); syncSelection(); return }
    if (e.button === 2 && S.selection.has(row.dataset.id)) return
    selectId(row.dataset.id, { additive: e.metaKey || e.ctrlKey, range: e.shiftKey })
  })
  body.addEventListener('dblclick', e => {
    const row = e.target.closest('.grid-row')
    if (row) api.revealInFinder(row.dataset.id)
  })
  body.addEventListener('contextmenu', async e => {
    e.preventDefault()
    const row = e.target.closest('.grid-row')
    if (!row) return
    if (!S.selection.has(row.dataset.id)) selectId(row.dataset.id)
    const r = S.rows.find(x => x.id === row.dataset.id)
    const action = await api.contextMenu('torrent', {
      count: S.selection.size, label: r?.label, sequential: r?.sequential
    })
    if (action) doAction(action)
  })

  // --- tabs
  $('#tabstrip').addEventListener('click', e => {
    const tab = e.target.closest('.tab')
    if (tab) setTab(tab.dataset.tab)
  })

  // --- file / tracker / peer context menus
  $('#pane-files').addEventListener('contextmenu', async e => {
    const tr = e.target.closest('tr[data-index]')
    if (!tr) return
    e.preventDefault()
    const index = Number(tr.dataset.index)
    const r = selectedRow()
    const action = await api.contextMenu('file', { priority: Number(tr.dataset.priority) })
    if (!action || !r) return
    if (action.startsWith('prio:')) await api.setFilePriority(r.id, index, Number(action.slice(5)))
    else if (action === 'open') await api.openItem(r.id, index)
    else if (action === 'reveal') await api.revealInFinder(r.id, index)
  })
  $('#pane-files').addEventListener('dblclick', async e => {
    const tr = e.target.closest('tr[data-index]')
    const r = selectedRow()
    if (tr && r) await api.openItem(r.id, Number(tr.dataset.index))
  })

  $('#pane-trackers').addEventListener('contextmenu', async e => {
    e.preventDefault()
    const tr = e.target.closest('tr[data-url]')
    const r = selectedRow()
    if (!r) return
    const action = await api.contextMenu('tracker', {})
    if (action === 'add') {
      const url = await openPrompt('Add Tracker', 'Tracker announce URL:', 'udp://tracker.opentrackr.org:1337/announce')
      if (url) await api.addTracker(r.id, url.trim())
    } else if (action === 'update') {
      await api.reannounce(r.id)
    } else if (action === 'copy' && tr) {
      await api.copyToClipboard(tr.dataset.url)
    }
  })

  $('#pane-peers').addEventListener('contextmenu', async e => {
    e.preventDefault()
    const tr = e.target.closest('tr[data-addr]')
    const r = selectedRow()
    if (!r) return
    const action = await api.contextMenu('peer', {})
    if (action === 'add') {
      const addr = await openPrompt('Add Peer', 'Peer address (ip:port):', '')
      if (addr) await api.addPeer(r.id, addr.trim())
    } else if (action === 'copy' && tr) {
      await api.copyToClipboard(tr.dataset.addr)
    }
  })

  // --- splitters
  dragSplit($('#vsplit'), 'x', () => parseInt(getComputedStyle(document.documentElement)
    .getPropertyValue('--sidebar-w')) || 186, v => {
    document.documentElement.style.setProperty('--sidebar-w', Math.max(120, Math.min(400, v)) + 'px')
  })
  dragSplit($('#hsplit'), 'y', () => parseInt(getComputedStyle(document.documentElement)
    .getPropertyValue('--detail-h')) || 232, v => {
    const max = window.innerHeight - 190
    document.documentElement.style.setProperty('--detail-h', Math.max(90, Math.min(max, v)) + 'px')
    renderDetail()
  }, -1)

  // --- keyboard
  window.addEventListener('keydown', e => {
    if (document.querySelector('#modal-root.open')) return
    const tag = document.activeElement?.tagName
    if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return

    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault()
      const rows = filteredRows()
      if (!rows.length) return
      const cur = rows.findIndex(r => r.id === S.anchor)
      const next = Math.max(0, Math.min(rows.length - 1, (cur < 0 ? 0 : cur + (e.key === 'ArrowDown' ? 1 : -1))))
      selectId(rows[next].id, { range: e.shiftKey })
      rowEls.get(rows[next].id)?.el.scrollIntoView({ block: 'nearest' })
    } else if (e.key === 'a' && (e.metaKey || e.ctrlKey)) {
      e.preventDefault()
      for (const r of filteredRows()) S.selection.add(r.id)
      syncSelection()
    } else if (e.key === 'Escape') {
      S.selection.clear(); syncSelection()
    } else if (e.key === 'Delete' || e.key === 'Backspace') {
      if (!e.metaKey && !e.ctrlKey) { e.preventDefault(); doAction(e.shiftKey ? 'remove-data' : 'remove') }
    } else if (e.key === ' ') {
      e.preventDefault()
      const r = selectedRow()
      if (r) doAction(r.state === 'paused' || r.state === 'stopped' ? 'start' : 'pause')
    } else if (e.key === 'Enter') {
      doAction('reveal')
    }
  })

  // --- dragging torrents onto a sidebar label
  const labelTarget = el => {
    const item = el?.closest?.('.tree-item')
    if (!item) return null
    if (item.dataset.drop === 'new') return item
    const cat = item.dataset.cat
    return (cat === 'nolabel' || cat?.startsWith('label:')) ? item : null
  }
  const clearDropTarget = () => {
    for (const el of document.querySelectorAll('#sidebar .drop-target')) {
      el.classList.remove('drop-target')
    }
  }

  $('#list-body').addEventListener('dragstart', e => {
    const row = e.target.closest('.grid-row')
    if (!row) return
    // Dragging a row outside the selection acts on just that row.
    if (!S.selection.has(row.dataset.id)) selectId(row.dataset.id, {})
    S.dragIds = [...S.selection]
    e.dataTransfer.effectAllowed = 'move'
    // Something must be set or Safari/WebKit cancels the drag outright.
    e.dataTransfer.setData('text/x-ztorrent', S.dragIds.join(','))

    if (S.dragIds.length > 1) {
      const ghost = document.createElement('div')
      ghost.className = 'drag-ghost'
      ghost.textContent = `${S.dragIds.length} torrents`
      document.body.appendChild(ghost)
      e.dataTransfer.setDragImage(ghost, 12, 12)
      setTimeout(() => ghost.remove(), 0)
    }
  })

  $('#list-body').addEventListener('dragend', () => { S.dragIds = null; clearDropTarget() })

  $('#sidebar').addEventListener('dragover', e => {
    if (!S.dragIds) return
    const item = labelTarget(e.target)
    if (!item) { clearDropTarget(); return }
    e.preventDefault()
    e.stopPropagation()
    e.dataTransfer.dropEffect = 'move'
    if (!item.classList.contains('drop-target')) {
      clearDropTarget()
      item.classList.add('drop-target')
    }
  })

  $('#sidebar').addEventListener('dragleave', e => {
    if (e.target.closest?.('.tree-item')?.contains(e.relatedTarget)) return
    if (!e.relatedTarget || !$('#sidebar').contains(e.relatedTarget)) clearDropTarget()
  })

  $('#sidebar').addEventListener('drop', async e => {
    const item = labelTarget(e.target)
    const ids = S.dragIds
    S.dragIds = null
    clearDropTarget()
    if (!item || !ids?.length) return
    e.preventDefault()
    e.stopPropagation()
    if (item.dataset.drop === 'new') {
      const name = await openPrompt('New Label', 'Label name:', '')
      if (name?.trim()) await api.setLabel(ids, name.trim())
      return
    }
    const cat = item.dataset.cat
    await api.setLabel(ids, cat === 'nolabel' ? '' : cat.slice(6))
  })

  // --- drag & drop of .torrent files and magnet links onto the window
  const stop = e => { e.preventDefault(); e.stopPropagation() }
  window.addEventListener('dragover', e => {
    // An internal row drag is not a file drop; don't flash the drop outline.
    if (S.dragIds) return
    stop(e)
    document.body.classList.add('dragover')
  })
  window.addEventListener('dragleave', e => {
    if (e.relatedTarget === null) document.body.classList.remove('dragover')
  })
  window.addEventListener('drop', async e => {
    document.body.classList.remove('dragover')
    if (S.dragIds) { S.dragIds = null; clearDropTarget(); return }
    stop(e)
    const files = [...(e.dataTransfer?.files || [])]
    const paths = files.map(f => api.pathForFile(f)).filter(p => p && p.endsWith('.torrent'))
    if (paths.length === 1) return openAddDialog(paths[0])
    if (paths.length > 1) return void api.addTorrentPaths(paths)

    const text = e.dataTransfer?.getData('text/plain')?.trim()
    if (text && (text.startsWith('magnet:') || /^https?:\/\/.+\.torrent/i.test(text))) {
      openAddDialog(text)
    }
  })

  window.addEventListener('resize', () => { if (S.showDetail) renderDetail() })
}

function dragSplit (el, axis, get, set, sign = 1) {
  el.addEventListener('mousedown', e => {
    e.preventDefault()
    const start = axis === 'x' ? e.clientX : e.clientY
    const base = get()
    const move = ev => set(base + sign * ((axis === 'x' ? ev.clientX : ev.clientY) - start))
    const up = () => {
      window.removeEventListener('mousemove', move)
      window.removeEventListener('mouseup', up)
      document.body.style.cursor = ''
    }
    document.body.style.cursor = axis === 'x' ? 'col-resize' : 'row-resize'
    window.addEventListener('mousemove', move)
    window.addEventListener('mouseup', up)
  })
}

/**
 * Exposed so the app can be driven from the outside (the --shot-eval capture
 * hook, and the File/Torrent menus which route through the same actions).
 * It grants nothing the toolbar does not already offer.
 */
window.ztorrentUI = {
  doAction,
  openAdd: openAddDialog,
  setTab,
  select: id => selectId(id),
  state: () => ({
    rows: S.rows.length,
    selection: [...S.selection],
    category: S.category,
    tab: S.tab,
    theme: document.documentElement.dataset.theme
  })
}

export { S, doAction }
