import { icon, TAG_SYMBOLS, TAG_COLORS, tagStyle } from './icons.js'
import * as fmt from './util.js'

const api = window.ztorrent
const root = document.getElementById('modal-root')

/* ═════════════════════════════════════════════════════════════ modal plumbing */

let closeCurrent = null

/**
 * Shows a modal and resolves with whatever `onOk` returns, or null on cancel.
 * `render` receives a scratch object it can hang state off.
 */
function modal ({ title, body, footer = '', width, onMount, onOk, okLabel = 'OK', cancelLabel = 'Cancel', hideOk }) {
  return new Promise(resolve => {
    root.innerHTML = `
      <div class="modal" ${width ? `style="width:${width}px"` : ''}>
        <div class="modal-title">${title}</div>
        <div class="modal-body">${body}</div>
        <div class="modal-foot">
          ${footer}
          <div class="grow"></div>
          <button data-role="cancel">${cancelLabel}</button>
          ${hideOk ? '' : `<button class="primary" data-role="ok">${okLabel}</button>`}
        </div>
      </div>`
    root.classList.add('open')

    const box = root.querySelector('.modal')
    const finish = value => {
      root.classList.remove('open')
      root.innerHTML = ''
      document.removeEventListener('keydown', onKey, true)
      closeCurrent = null
      resolve(value)
    }
    closeCurrent = () => finish(null)

    const confirm = async () => {
      if (!onOk) return finish(true)
      const okBtn = box.querySelector('[data-role="ok"]')
      if (okBtn) okBtn.disabled = true
      try {
        const out = await onOk(box)
        if (out === false) { if (okBtn) okBtn.disabled = false; return }
        finish(out)
      } catch (err) {
        if (okBtn) okBtn.disabled = false
        const slot = box.querySelector('[data-role="error"]')
        if (slot) slot.textContent = err.message
        else throw err
      }
    }

    box.querySelector('[data-role="cancel"]').onclick = () => finish(null)
    const ok = box.querySelector('[data-role="ok"]')
    if (ok) ok.onclick = confirm

    const onKey = e => {
      if (e.key === 'Escape') { e.stopPropagation(); finish(null) }
      if (e.key === 'Enter' && !e.shiftKey && e.target.tagName !== 'TEXTAREA') {
        e.stopPropagation(); e.preventDefault(); if (ok) confirm()
      }
    }
    document.addEventListener('keydown', onKey, true)

    root.onmousedown = e => { if (e.target === root) finish(null) }
    onMount?.(box, finish)

    const first = box.querySelector('input:not([type=checkbox]):not([type=radio]), textarea, select')
    first?.focus()
    if (first?.select) first.select()
  })
}

export function closeModal () { closeCurrent?.() }

/* ═══════════════════════════════════════════════════════════ generic prompt */

export function openPrompt (title, label, value = '') {
  return modal({
    title,
    width: 400,
    body: `<div class="frow wide">
             <label>${fmt.esc(label)}</label>
             <input type="text" data-role="value" value="${fmt.esc(value)}">
           </div>`,
    onOk: box => {
      const v = box.querySelector('[data-role="value"]').value
      return v.trim() ? v : false
    }
  })
}

/* ════════════════════════════════════════════════════════════ Add Torrent */

/**
 * The "Add New Torrent" sheet: destination, contents, and start options.
 * `defaultPath` overrides the offered folder for this sheet only; without one
 * we offer wherever the last torrent went, falling back to the default folder.
 */
export async function openAddDialog (source, defaultPath = '') {
  const settings = await api.getSettings()
  const labels = await api.getLabels()
  const info = await api.inspectTorrent(source)

  if (!info.ok) {
    return modal({
      title: 'Add Torrent',
      width: 420,
      hideOk: true,
      cancelLabel: 'Close',
      body: `<div class="err-text">Could not read this torrent.</div>
             <div style="margin-top:6px;color:var(--ink-dim)">${fmt.esc(info.error)}</div>`
    })
  }

  const saveIn = defaultPath || settings.lastSavePath || settings.downloadPath
  const isMagnet = typeof source === 'string' && source.startsWith('magnet:')
  const files = info.files || []
  const hasFiles = files.length > 0

  const fileRows = files.map(f => `
    <tr data-index="${f.index}">
      <td style="width:24px"><input type="checkbox" data-file="${f.index}" checked></td>
      <td title="${fmt.esc(f.path)}">${fmt.esc(f.path || f.name)}</td>
      <td class="num" style="width:88px">${fmt.bytes(f.length)}</td>
    </tr>`).join('')

  const body = `
    <fieldset>
      <legend>Save In</legend>
      <div class="frow wide">
        <div class="path-row">
          <input type="text" data-role="path" value="${fmt.esc(saveIn)}">
          <button class="small" data-role="browse">Browse…</button>
        </div>
      </div>
      <div class="frow">
        <label>Label:</label>
        <div class="inline">
          <input type="text" data-role="label" list="label-list" placeholder="(none)" style="width:170px">
          <datalist id="label-list">${labels.map(l => `<option value="${fmt.esc(l)}">`).join('')}</datalist>
        </div>
      </div>
    </fieldset>

    <fieldset>
      <legend>Torrent Contents</legend>
      <div class="frow wide" style="margin-bottom:6px">
        <div class="inline wrap" style="justify-content:space-between">
          <span><b>${fmt.esc(info.name)}</b></span>
          <span class="dim" data-role="sizesum">${fmt.bytes(info.length)}</span>
        </div>
      </div>
      ${hasFiles ? `
        <div class="filelist">
          <table class="dtable">
            <thead><tr><th style="width:24px"><input type="checkbox" data-role="all" checked></th>
              <th>Name</th><th class="num" style="width:88px">Size</th></tr></thead>
            <tbody>${fileRows}</tbody>
          </table>
        </div>` : `
        <div style="color:var(--ink-dim)">
          ${isMagnet
            ? 'This is a magnet link — the file list arrives once metadata is fetched from the swarm.'
            : 'Single-file torrent.'}
        </div>`}
      <div class="frow" style="margin-top:8px">
        <label>Info Hash:</label>
        <div style="font:10px var(--mono);user-select:text">${fmt.esc(info.infoHash)}</div>
      </div>
      ${info.comment ? `<div class="frow"><label>Comment:</label><div>${fmt.esc(info.comment)}</div></div>` : ''}
      <div class="frow"><label>Trackers:</label><div>${info.announce.length} announce URL(s)</div></div>
    </fieldset>

    <div class="inline wrap" style="gap:16px">
      <label class="ck"><input type="checkbox" data-role="start" ${settings.startTorrentsAutomatically ? 'checked' : ''}> Start torrent</label>
      <label class="ck"><input type="checkbox" data-role="seq" ${settings.sequentialDownload ? 'checked' : ''}> Download sequentially</label>
    </div>
    <div data-role="error" class="err-text" style="margin-top:6px"></div>`

  return modal({
    title: 'Add New Torrent',
    width: 560,
    okLabel: 'OK',
    body,
    onMount: box => {
      box.querySelector('[data-role="browse"]').onclick = async () => {
        const dir = await api.chooseFolder('Choose Download Folder')
        if (dir) box.querySelector('[data-role="path"]').value = dir
      }
      const all = box.querySelector('[data-role="all"]')
      const boxes = () => [...box.querySelectorAll('[data-file]')]
      const refresh = () => {
        const total = boxes().filter(b => b.checked)
          .reduce((s, b) => s + files[Number(b.dataset.file)].length, 0)
        box.querySelector('[data-role="sizesum"]').textContent =
          total === info.length ? fmt.bytes(info.length) : `${fmt.bytes(total)} of ${fmt.bytes(info.length)}`
      }
      if (all) {
        all.onchange = () => { boxes().forEach(b => { b.checked = all.checked }); refresh() }
        box.addEventListener('change', e => { if (e.target.dataset.file !== undefined) refresh() })
      }
    },
    onOk: async box => {
      const savePath = box.querySelector('[data-role="path"]').value.trim()
      if (!savePath) throw new Error('Choose a download folder.')
      const start = box.querySelector('[data-role="start"]').checked
      const sequential = box.querySelector('[data-role="seq"]').checked
      const label = box.querySelector('[data-role="label"]').value.trim()

      const priorities = {}
      let wanted = null
      const boxes = [...box.querySelectorAll('[data-file]')]
      if (boxes.length) {
        wanted = []
        for (const b of boxes) {
          const i = Number(b.dataset.file)
          priorities[i] = b.checked ? 1 : 0
          if (b.checked) wanted.push(i)
        }
        if (!wanted.length) throw new Error('Select at least one file.')
      }

      const res = await api.addInspected(source, {
        savePath, label, sequential, paused: !start, wanted, priorities
      })
      if (res?.duplicate) throw new Error('This torrent is already in the list.')
      // Remember where it went, so the next torrent is offered the same folder.
      if (savePath !== settings.lastSavePath) await api.setSettings({ lastSavePath: savePath })
      return res
    }
  })
}

/* ═════════════════════════════════════════════════════════ Add torrent by URL */

export async function openUrlDialog () {
  const settings = await api.getSettings()
  const url = await modal({
    title: 'Add Torrent from URL',
    width: 480,
    okLabel: 'Continue',
    body: `
      <div class="frow wide">
        <label>Enter a magnet link, an info hash, or an http(s) link to a .torrent file:</label>
        <input type="text" data-role="url" placeholder="magnet:?xt=urn:btih:… or https://example.org/file.torrent">
      </div>
      <div class="frow" style="margin-top:8px">
        <label>Save In:</label>
        <div class="path-row">
          <input type="text" data-role="path" value="${fmt.esc(settings.lastSavePath || settings.downloadPath)}">
          <button class="small" data-role="browse">Browse…</button>
        </div>
      </div>
      <div data-role="error" class="err-text" style="margin-top:6px"></div>`,
    onMount: box => {
      box.querySelector('[data-role="browse"]').onclick = async () => {
        const dir = await api.chooseFolder('Choose Download Folder')
        if (dir) box.querySelector('[data-role="path"]').value = dir
      }
      // Offer whatever magnet link is already on the clipboard, like µTorrent does.
      navigator.clipboard?.readText?.().then(t => {
        const input = box.querySelector('[data-role="url"]')
        if (t && !input.value && (t.startsWith('magnet:') || /^https?:\/\/\S+\.torrent/i.test(t.trim()))) {
          input.value = t.trim()
          input.select()
        }
      }).catch(() => {})
    },
    onOk: box => {
      let v = box.querySelector('[data-role="url"]').value.trim()
      if (!v) throw new Error('Enter a link.')
      if (/^[0-9a-f]{40}$/i.test(v)) v = `magnet:?xt=urn:btih:${v.toLowerCase()}`
      return { url: v, savePath: box.querySelector('[data-role="path"]').value.trim() }
    }
  })
  if (!url) return
  return openAddDialog(url.url, url.savePath)
}

/* ═══════════════════════════════════════════════════════════ Customise Label */

/**
 * Picks the symbol and colour a label wears in the sidebar. Both grids are
 * one tap with the row above them updating as you go, so the sheet shows the
 * result rather than describing it; there is nothing to read and nothing to
 * undo but Cancel. Resolves { symbol, color }, or null.
 */
export function openLabelStyle (name, current) {
  const start = tagStyle(current)

  const swatch = c => `
    <button type="button" data-color="${c}" title="${c[0].toUpperCase() + c.slice(1)}"
            aria-pressed="${c === start.color}" style="--sw: var(--tag-${c})"></button>`

  const symbol = sym => `
    <button type="button" data-symbol="${sym}" title="${sym}"
            aria-pressed="${sym === start.symbol}">${icon(sym)}</button>`

  return modal({
    title: 'Customize Label',
    width: 400,
    body: `
      <div class="tag-preview" data-role="preview" data-tag="${start.color}">
        ${icon(start.symbol)}<span class="lbl">${fmt.esc(name)}</span>
      </div>
      <div class="tag-heading">Color</div>
      <div class="tag-colors" data-role="colors">${TAG_COLORS.map(swatch).join('')}</div>
      <div class="tag-heading">Symbol</div>
      <div class="tag-symbols" data-role="symbols">${TAG_SYMBOLS.map(symbol).join('')}</div>`,
    onMount: box => {
      const preview = box.querySelector('[data-role="preview"]')
      const pick = (group, attr, apply) => {
        box.querySelector(`[data-role="${group}"]`).addEventListener('click', e => {
          const btn = e.target.closest(`[data-${attr}]`)
          if (!btn) return
          for (const b of box.querySelectorAll(`[data-role="${group}"] [data-${attr}]`)) {
            b.setAttribute('aria-pressed', String(b === btn))
          }
          apply(btn.dataset[attr])
        })
      }
      pick('colors', 'color', c => { preview.dataset.tag = c })
      pick('symbols', 'symbol', sym => {
        preview.querySelector('svg').outerHTML = icon(sym)
      })
    },
    onOk: box => ({
      color: box.querySelector('[data-role="colors"] [aria-pressed="true"]').dataset.color,
      symbol: box.querySelector('[data-role="symbols"] [aria-pressed="true"]').dataset.symbol
    })
  })
}

/* ══════════════════════════════════════════════════════════ Create Torrent */

const PIECE_SIZES = [
  ['Auto', 0],
  ['16 kB', 16384], ['32 kB', 32768], ['64 kB', 65536], ['128 kB', 131072],
  ['256 kB', 262144], ['512 kB', 524288], ['1 MB', 1048576], ['2 MB', 2097152],
  ['4 MB', 4194304], ['8 MB', 8388608], ['16 MB', 16777216]
]

const DEFAULT_TRACKERS = [
  'udp://tracker.opentrackr.org:1337/announce',
  'udp://open.demonii.com:1337/announce',
  'udp://tracker.torrent.eu.org:451/announce'
].join('\n')

export function openCreateDialog () {
  return modal({
    title: 'Create New Torrent',
    width: 560,
    okLabel: 'Create and Save As…',
    body: `
      <fieldset>
        <legend>Select Source</legend>
        <div class="frow wide">
          <div class="path-row">
            <input type="text" data-role="src" placeholder="Choose a file or folder to share">
            <button class="small" data-role="pick-file">Add File…</button>
            <button class="small" data-role="pick-dir">Add Folder…</button>
          </div>
        </div>
      </fieldset>

      <fieldset>
        <legend>Torrent Properties</legend>
        <div class="frow wide" style="margin-bottom:8px">
          <label>Trackers (one per line, blank line separates tiers):</label>
          <textarea data-role="trackers" spellcheck="false">${DEFAULT_TRACKERS}</textarea>
        </div>
        <div class="frow wide" style="margin-bottom:8px">
          <label>Web seeds (optional, one URL per line):</label>
          <textarea data-role="webseeds" spellcheck="false" style="min-height:40px"></textarea>
        </div>
        <div class="frow">
          <label>Comment:</label>
          <input type="text" data-role="comment" style="width:100%">
        </div>
        <div class="frow">
          <label>Piece size:</label>
          <select data-role="piece">
            ${PIECE_SIZES.map(([l, v]) => `<option value="${v}">${l}</option>`).join('')}
          </select>
        </div>
      </fieldset>

      <div class="inline wrap" style="gap:16px">
        <label class="ck"><input type="checkbox" data-role="private"> Private torrent (disable DHT and PEX)</label>
        <label class="ck"><input type="checkbox" data-role="seed" checked> Start seeding</label>
      </div>
      <div data-role="error" class="err-text" style="margin-top:6px"></div>
      <div data-role="status" style="margin-top:6px;color:var(--ink-dim)"></div>`,
    onMount: box => {
      box.querySelector('[data-role="pick-file"]').onclick = async () => {
        const p = await api.chooseFile('Choose a File to Share')
        if (p) box.querySelector('[data-role="src"]').value = p
      }
      box.querySelector('[data-role="pick-dir"]').onclick = async () => {
        const p = await api.chooseFolder('Choose a Folder to Share')
        if (p) box.querySelector('[data-role="src"]').value = p
      }
    },
    onOk: async box => {
      const inputPath = box.querySelector('[data-role="src"]').value.trim()
      if (!inputPath) throw new Error('Choose a file or folder first.')

      const name = inputPath.split('/').filter(Boolean).pop()
      const dest = await api.saveFileDialog(`${name}.torrent`,
        [{ name: 'Torrent Files', extensions: ['torrent'] }])
      if (!dest) return false

      box.querySelector('[data-role="status"]').textContent = 'Hashing… this can take a moment for large folders.'

      const res = await api.createTorrent({
        inputPath,
        outputPath: dest,
        name,
        comment: box.querySelector('[data-role="comment"]').value.trim(),
        trackers: box.querySelector('[data-role="trackers"]').value.split('\n'),
        webSeeds: box.querySelector('[data-role="webseeds"]').value.split('\n'),
        pieceLength: Number(box.querySelector('[data-role="piece"]').value),
        private: box.querySelector('[data-role="private"]').checked,
        startSeeding: box.querySelector('[data-role="seed"]').checked
      })
      if (!res.ok) throw new Error(res.error)
      return res
    }
  })
}

/* ═══════════════════════════════════════════════════════════════ Preferences */

const PREF_PAGES = [
  { id: 'general',    label: 'General',     ic: 'preferences' },
  { id: 'dirs',       label: 'Directories', ic: 'folder' },
  { id: 'connection', label: 'Connection',  ic: 'tracker' },
  { id: 'bandwidth',  label: 'Bandwidth',   ic: 'speed' },
  { id: 'queueing',   label: 'Queueing',    ic: 'files' },
  { id: 'ui',         label: 'Appearance',  ic: 'info' }
]

export async function openPreferences (settings, apply) {
  const interfaces = await api.getInterfaces().catch(() => [])
  const ck = (key, label) =>
    `<label class="ck"><input type="checkbox" data-k="${key}" ${settings[key] ? 'checked' : ''}> ${label}</label>`
  const txt = (key, label, placeholder = '') => `
    <div class="frow">
      <label>${label}</label>
      <input type="text" data-k="${key}" value="${fmt.esc(String(settings[key] ?? ''))}"
             placeholder="${placeholder}" spellcheck="false">
    </div>`
  const pw = (key, label) => `
    <div class="frow">
      <label>${label}</label>
      <input type="password" data-k="${key}" value="${fmt.esc(String(settings[key] ?? ''))}">
    </div>`
  const sel = (key, label, opts, hint = '') => `
    <div class="frow">
      <label>${label}</label>
      <select data-k="${key}" data-num>
        ${opts.map(([v, t]) =>
          `<option value="${v}" ${Number(settings[key]) === v ? 'selected' : ''}>${t}</option>`).join('')}
      </select>
    </div>
    ${hint ? `<div class="frow"><div></div><div class="hint">${hint}</div></div>` : ''}`
  const num = (key, label, suffix = '', min = 0) => `
    <div class="frow">
      <label>${label}</label>
      <div class="inline"><input type="number" data-k="${key}" min="${min}" value="${settings[key]}">
        <span class="dim">${suffix}</span></div>
    </div>`

  const body = `
    <div class="prefs">
      <div class="prefs-nav">
        ${PREF_PAGES.map((p, i) => `<div class="pn ${i === 0 ? 'active' : ''}" data-page="${p.id}">
            ${icon(p.ic)}<span>${p.label}</span></div>`).join('')}
      </div>
      <div>
        <div class="prefs-page active" data-page="general">
          <fieldset><legend>When adding torrents</legend>
            <div class="frow wide">${ck('startTorrentsAutomatically', 'Start torrents automatically')}</div>
            <div class="frow wide">${ck('askWhereToSave', 'Show the Add Torrent dialog')}</div>
            <div class="frow wide">${ck('sequentialDownload', 'Download pieces in order by default')}</div>
            <div class="frow wide">${ck('partFiles', 'Append .part to incomplete files')}</div>
            <div class="frow"><span></span><span class="hint">Renamed once the torrent finishes, so unfinished downloads are never mistaken for complete ones.</span></div>
          </fieldset>
          <fieldset><legend>Notifications</legend>
            <div class="frow wide">${ck('notifyOnComplete', 'Show a notification when a download finishes')}</div>
            <div class="frow wide">${ck('confirmOnDelete', 'Confirm before removing a torrent')}</div>
            <div class="frow wide">${ck('showSpeedInDock', 'Show download progress on the Dock icon')}</div>
          </fieldset>
        </div>

        <div class="prefs-page" data-page="dirs">
          <fieldset><legend>Location of downloaded files</legend>
            <div class="frow wide">
              <label>Put new downloads in:</label>
              <div class="path-row">
                <input type="text" data-k="downloadPath" value="${fmt.esc(settings.downloadPath)}">
                <button class="small" data-role="browse-dl">Browse…</button>
              </div>
            </div>
          </fieldset>
        </div>

        <div class="prefs-page" data-page="connection">
          <fieldset><legend>Listening port</legend>
            <div class="frow wide">${ck('randomizePort', 'Randomize the port each time ztorrent starts')}</div>
            ${num('listenPort', 'Port used for incoming connections:', '', 0)}
            <div class="frow wide">${ck('enableUPnP', 'Map the port with UPnP / NAT-PMP')}</div>
          </fieldset>
          <fieldset><legend>Proxy</legend>
            <div class="frow wide">${ck('proxyEnabled', 'Route traffic through a SOCKS5 proxy')}</div>
            ${txt('proxyHost', 'Proxy host:', '127.0.0.1')}
            ${num('proxyPort', 'Proxy port:', '', 1)}
            ${txt('proxyUsername', 'Username:', '(optional)')}
            ${pw('proxyPassword', 'Password:')}
            <div class="frow"><div></div><div class="hint">Covers tracker announces, web seeds and
              outgoing peer connections. Anything that cannot be routed is switched off rather than
              sent around the proxy: DHT, local discovery, µTP, port mapping and udp:// trackers all
              stop while this is on. Takes effect after a restart.</div></div>
          </fieldset>
          <fieldset><legend>Network interface</legend>
            <div class="frow">
              <label>Send traffic from:</label>
              <select data-k="bindInterface">
                <option value="" ${!settings.bindInterface ? 'selected' : ''}>Any (follow the routing table)</option>
                ${interfaces.map(i => `<option value="${fmt.esc(i.name)}"
                  ${settings.bindInterface === i.name ? 'selected' : ''}>${fmt.esc(i.name)} — ${fmt.esc(i.address)}</option>`).join('')}
                ${settings.bindInterface && !interfaces.some(i => i.name === settings.bindInterface)
                  ? `<option value="${fmt.esc(settings.bindInterface)}" selected>${fmt.esc(settings.bindInterface)} — not present</option>` : ''}
              </select>
            </div>
            <div class="frow"><div></div><div class="hint">Pin every outgoing connection to one
              interface — a VPN's, typically. If it goes away, connections fail instead of falling
              back to your normal one, and resume by themselves when it returns. Local discovery,
              µTP, port mapping and udp:// trackers stop while this is set; DHT keeps working,
              bound to the same interface. Takes effect after a restart.</div></div>
          </fieldset>
          <fieldset><legend>Protocol encryption</legend>
            ${sel('encryption', 'Peer connections:', [
              [0, 'Disabled — plaintext handshakes'],
              [1, 'Enabled — encrypt when the peer supports it'],
              [2, 'Required — refuse peers that will not encrypt']
            ], 'Hides the handshake from traffic inspection. It does not hide ' +
               'tracker or DHT activity, and your address is still public to the swarm.')}
          </fieldset>
          <fieldset><legend>Peer discovery</legend>
            <div class="frow wide">${ck('enableDHT', 'Enable DHT (distributed hash table)')}</div>
            <div class="frow wide">${ck('enablePEX', 'Enable peer exchange')}</div>
            <div class="frow wide">${ck('enableLSD', 'Enable local peer discovery')}</div>
            <div class="frow"><div></div><div class="hint">Local discovery broadcasts each torrent's
              infohash in the clear to every device on your network, and only finds peers on that
              same network. Off by default.</div></div>
            <div class="frow wide">${ck('enableUTP', 'Enable µTP (micro transport protocol)')}</div>
            <div class="frow"><div></div><div class="hint">Discovery changes take effect after a restart.</div></div>
          </fieldset>
        </div>

        <div class="prefs-page" data-page="bandwidth">
          <fieldset><legend>Global rate limits</legend>
            ${num('maxDownloadRate', 'Maximum download rate:', 'kB/s  (0 = unlimited)')}
            ${num('maxUploadRate', 'Maximum upload rate:', 'kB/s  (0 = unlimited)')}
          </fieldset>
          <fieldset><legend>Alternate rate limits</legend>
            ${num('altDownloadRate', 'Alternate download rate:', 'kB/s')}
            ${num('altUploadRate', 'Alternate upload rate:', 'kB/s')}
            <div class="frow wide">${ck('altSpeedEnabled', 'Use alternate limits now')}</div>
          </fieldset>
          <fieldset><legend>Number of connections</legend>
            ${num('globalMaxConnections', 'Global maximum connections:', '', 10)}
            ${num('maxUploadSlots', 'Upload slots per torrent:', '', 1)}
          </fieldset>
        </div>

        <div class="prefs-page" data-page="queueing">
          <fieldset><legend>Queue settings</legend>
            ${num('maxActiveTorrents', 'Maximum active torrents:', '', 1)}
            ${num('maxActiveDownloads', 'Maximum active downloads:', '', 1)}
          </fieldset>
          <fieldset><legend>Seeding goal</legend>
            ${num('seedRatioLimit', 'Seed until ratio reaches:', '%  (0 = forever)')}
            ${num('seedTimeLimit', 'Seed for at least:', 'minutes  (0 = forever)')}
          </fieldset>
        </div>

        <div class="prefs-page" data-page="ui">
          <fieldset><legend>Theme</legend>
            <div class="frow">
              <label>Appearance:</label>
              <select data-k="theme">
                <option value="classic" ${settings.theme === 'classic' ? 'selected' : ''}>Light</option>
                <option value="graphite" ${settings.theme === 'graphite' ? 'selected' : ''}>Dark</option>
              </select>
            </div>
          </fieldset>
        </div>
      </div>
    </div>`

  return modal({
    title: 'Preferences',
    width: 620,
    okLabel: 'Apply',
    body,
    onMount: box => {
      box.querySelector('.prefs-nav').onclick = e => {
        const pn = e.target.closest('.pn')
        if (!pn) return
        for (const el of box.querySelectorAll('.pn')) el.classList.toggle('active', el === pn)
        for (const el of box.querySelectorAll('.prefs-page')) {
          el.classList.toggle('active', el.dataset.page === pn.dataset.page)
        }
      }
      box.querySelector('[data-role="browse-dl"]').onclick = async () => {
        const dir = await api.chooseFolder('Choose Download Folder')
        if (dir) box.querySelector('[data-k="downloadPath"]').value = dir
      }
      // The proxy switches these off in the engine whatever the boxes say, so
      // grey them out rather than letting the sheet imply otherwise.
      const proxyBox = box.querySelector('[data-k="proxyEnabled"]')
      const bindSel = box.querySelector('[data-k="bindInterface"]')
      const grey = (el, off) => {
        el.disabled = off
        el.closest('label').style.opacity = off ? '.45' : ''
      }
      const syncSuppressed = () => {
        const proxied = proxyBox.checked
        const pinned = proxied || !!bindSel.value
        // DHT survives a bind -- its socket can be pinned too -- but not a proxy.
        grey(box.querySelector('[data-k="enableDHT"]'), proxied)
        for (const k of ['enableLSD', 'enableUTP', 'enableUPnP']) {
          grey(box.querySelector(`[data-k="${k}"]`), pinned)
        }
      }
      proxyBox.onchange = syncSuppressed
      bindSel.onchange = syncSuppressed
      syncSuppressed()

      // Live-preview the theme while the sheet is open.
      box.querySelector('[data-k="theme"]').onchange = e => {
        document.documentElement.dataset.theme = e.target.value
      }
    },
    onOk: box => {
      const patch = {}
      for (const el of box.querySelectorAll('[data-k]')) {
        const k = el.dataset.k
        if (el.type === 'checkbox') patch[k] = el.checked
        else if (el.type === 'number') patch[k] = Number(el.value) || 0
        else if (el.dataset.num !== undefined) patch[k] = Number(el.value)
        else patch[k] = el.value
      }
      apply(patch)
      return patch
    }
  }).then(res => {
    if (!res) document.documentElement.dataset.theme = settings.theme || 'classic'
    return res
  })
}

/* ═══════════════════════════════════════════════════════════════ Properties */

export function openProperties (row, details) {
  if (!row) return
  const line = (k, v) => `<dt>${k}</dt><dd class="sel">${fmt.esc(v)}</dd>`
  return modal({
    title: `Properties — ${fmt.esc(row.name)}`,
    width: 520,
    hideOk: true,
    cancelLabel: 'Close',
    body: `<dl class="gen-grid" style="grid-template-columns:130px 1fr">
      ${line('Name', row.name)}
      ${line('Info Hash', row.infoHash)}
      ${line('Save Path', row.savePath)}
      ${line('Total Size', fmt.bytes(row.size))}
      ${line('Selected Size', fmt.bytes(row.wantedSize))}
      ${line('Pieces', details ? `${details.pieceCount} × ${fmt.bytes(details.pieceLength)}` : '—')}
      ${line('Private', details ? (details.private ? 'Yes' : 'No') : '—')}
      ${line('Comment', details?.comment || '—')}
      ${line('Created By', details?.createdBy || '—')}
      ${line('Created On', details?.createdOn ? fmt.datetime(details.createdOn) : '—')}
      ${line('Added On', fmt.datetime(row.addedOn))}
      ${line('Completed On', row.completedOn ? fmt.datetime(row.completedOn) : '—')}
      ${line('Downloaded', fmt.bytes(row.downloaded))}
      ${line('Uploaded', fmt.bytes(row.uploaded))}
      ${line('Ratio', fmt.ratio(row.ratio))}
      ${line('Label', row.label || '—')}
      ${line('Magnet URI', row.magnetURI || '—')}
    </dl>`
  })
}
