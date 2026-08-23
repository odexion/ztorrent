import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import assert from 'node:assert'
import { Store } from '../electron/store.js'

const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'ztor-'))
const codec = {  // stand-in for safeStorage
  encrypt: v => 'ENC(' + Buffer.from(v).toString('base64') + ')',
  decrypt: s => Buffer.from(String(s).slice(4, -1), 'base64').toString()
}

let store = new Store(dir, codec)
store.patchSettings({ proxyEnabled: true, proxyHost: 'p.example', proxyPassword: 'hunter2' })
store.flush()

const onDisk = JSON.parse(fs.readFileSync(path.join(dir, 'ztorrent-state.json'), 'utf8'))
assert.equal(onDisk.settings.proxyPassword, undefined, 'plaintext password not written')
assert.ok(onDisk.settings.proxyPasswordEnc.startsWith('ENC('), 'sealed value written')
assert.ok(!JSON.stringify(onDisk).includes('hunter2'), 'password appears nowhere in the file')

store = new Store(dir, codec)
assert.equal(store.settings.proxyPassword, 'hunter2', 'round-trips on reload')
assert.equal(store.settings.proxyPasswordEnc, undefined, 'sealed key not left in memory')
assert.equal(store.settings.proxyHost, 'p.example', 'other settings survive')

// A codec that throws on decrypt (keychain denied) must not break startup, and
// must not destroy the sealed password on the next save.
const denied = new Store(dir, { encrypt: codec.encrypt, decrypt: () => { throw new Error('denied') } })
assert.equal(denied.settings.proxyPassword, '', 'unreadable password reads as empty')
denied.patchSettings({ maxUploadRate: 50 })
denied.flush()
assert.ok(JSON.parse(fs.readFileSync(path.join(dir, 'ztorrent-state.json'), 'utf8'))
  .settings.proxyPasswordEnc.startsWith('ENC('), 'sealed password survived the denied session')
assert.equal(new Store(dir, codec).settings.proxyPassword, 'hunter2', 'still there afterwards')

// Clearing it deliberately really does clear it.
const clearing = new Store(dir, codec)
clearing.patchSettings({ proxyPassword: '' })
clearing.flush()
const after = JSON.parse(fs.readFileSync(path.join(dir, 'ztorrent-state.json'), 'utf8'))
assert.equal(after.settings.proxyPasswordEnc, undefined, 'sealed value removed')
assert.equal(after.settings.proxyPassword, undefined, 'and not written in the clear')

// No codec at all (some Linux desktops): works, stays plaintext, nothing lost.
const plain = new Store(dir, null)
plain.patchSettings({ proxyPassword: 'fallback' })
plain.flush()
assert.equal(JSON.parse(fs.readFileSync(path.join(dir, 'ztorrent-state.json'), 'utf8'))
  .settings.proxyPassword, 'fallback')

fs.rmSync(dir, { recursive: true, force: true })
console.log('secrets: all assertions passed')
