// Does a proxied client still reach 'listening' (and therefore start torrents)
// after the inbound listeners are closed?
import { Engine } from '../electron/engine.js'
import { DEFAULT_SETTINGS } from '../electron/store.js'
import net from 'node:net'
import assert from 'node:assert'

const settings = { ...DEFAULT_SETTINGS, proxyEnabled: true, proxyHost: '127.0.0.1', proxyPort: 1080 }
const store = { settings, data: {}, set () {}, patchSettings (p) { return Object.assign(settings, p) } }
const e = new Engine(store)
e.start()

const listening = await new Promise(r => {
  e.client.once('listening', () => r(true))
  setTimeout(() => r(false), 4000)
})
assert.ok(listening, 'client reached listening')
assert.ok(e.client.listening, 'and set the flag torrents wait on')

await new Promise(r => setImmediate(r))
const port = e.client.torrentPort
assert.ok(port > 0, 'a port was assigned before the close: ' + port)

// Nothing should accept on it any more.
const refused = await new Promise(r => {
  const c = net.connect({ host: '127.0.0.1', port })
  c.once('connect', () => { c.destroy(); r(false) })
  c.once('error', () => r(true))
  setTimeout(() => { c.destroy(); r(false) }, 1500)
})
assert.ok(refused, `inbound port ${port} still accepting connections`)
console.log(`listening reached, port ${port} assigned, inbound refused — as intended`)
await e.destroy().catch(() => {})
process.exit(0)
