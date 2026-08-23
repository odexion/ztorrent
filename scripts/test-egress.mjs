import net from 'node:net'
import http from 'node:http'
import assert from 'node:assert'
import { startSocks } from './socks-server.mjs'
import { proxyConfig, egressPolicy, useEgress, guardedOptions, samePolicy, listInterfaces, closeInbound }
  from '../electron/egress.js'
import { Client as DHT } from 'bittorrent-dht'
import Torrent from 'webtorrent/lib/torrent.js'

const results = []
const check = async (name, fn) => {
  try { await fn(); results.push('PASS'); console.log('  ok   ' + name) }
  catch (e) { results.push('FAIL'); console.log('  FAIL ' + name + '\n       ' + (e.message || e)) }
}

/** The server's accept can land after the client's connect event, so poll. */
const waitFor = async (fn, what) => {
  for (let i = 0; i < 100; i++) {
    if (fn()) return
    await new Promise(r => setTimeout(r, 20))
  }
  throw new Error('timed out waiting for ' + what)
}

/** Runs one outgoing dial exactly the way Torrent._drain does in production. */
function dialPeer (port, host = '127.0.0.1') {
  const peer = { type: 'tcpOutgoing', addr: `${host}:${port}`, onConnect () {}, destroy () {}, startConnectTimeout () {} }
  Torrent.prototype._drain.call({
    _debug () {}, destroyed: false, paused: false, _numConns: 0, _numPending: 0,
    client: { maxConns: 10, utp: false }, _queue: [peer], _numQueued: 1, numPeers: 0,
    _addPeer: () => null, _isIPv4: () => true, removePeer () {}
  })
  return peer.conn
}

const socks = startSocks({ user: 'zaf', pass: 'hunter2' })
const socksPort = await socks.listen()
const seenFrom = []
const echo = net.createServer(c => { seenFrom.push(c.remoteAddress.replace(/^::ffff:/, '')); c.pipe(c) })
await new Promise(r => echo.listen(0, r))   // all interfaces, so bind tests can reach it
const echoPort = echo.address().port
const webSeenFrom = []
const web = http.createServer((req, res) => {
  webSeenFrom.push(req.socket.remoteAddress.replace(/^::ffff:/, ''))
  res.end('announce-ok:' + req.url)
})
await new Promise(r => web.listen(0, r))
const webPort = web.address().port

const settings = { proxyEnabled: true, proxyHost: '127.0.0.1', proxyPort: socksPort,
                   proxyUsername: 'zaf', proxyPassword: 'hunter2' }
const cfg = egressPolicy(settings)
useEgress(cfg)
console.log('\nSOCKS5 on 127.0.0.1:%d  echo:%d  http:%d\n', socksPort, echoPort, webPort)

const nic = listInterfaces()[0]
console.log(nic ? `binding tests will use ${nic.name} (${nic.address})\n`
                : 'no non-internal IPv4 interface -- binding tests will be skipped\n')

await check('proxyConfig rejects disabled or incomplete settings', async () => {
  assert.equal(proxyConfig({ ...settings, proxyEnabled: false }), null)
  assert.equal(proxyConfig({ ...settings, proxyHost: '  ' }), null)
  assert.equal(proxyConfig({ ...settings, proxyPort: 0 }), null)
  assert.ok(samePolicy(cfg, egressPolicy(settings)))
  assert.ok(!samePolicy(cfg, egressPolicy({ ...settings, proxyPort: 9999 })))
  assert.ok(!samePolicy(cfg, egressPolicy({ ...settings, bindInterface: 'utun9' })))
  assert.equal(egressPolicy({ proxyEnabled: false, bindInterface: '' }), null)
})

await check('guardedOptions disables every unroutable channel', async () => {
  assert.deepEqual(guardedOptions(cfg),
    { lsd: false, utp: false, natUpnp: false, natPmp: false, dht: false })
  // A bind can pin the DHT socket, so DHT survives it.
  assert.deepEqual(guardedOptions({ proxy: null, bind: 'utun4' }),
    { lsd: false, utp: false, natUpnp: false, natPmp: false })
  assert.deepEqual(guardedOptions(null), {})
})

await check('outgoing peer connection is dialled through the proxy, and carries data', async () => {
  const before = socks.stats.connects.length
  const conn = dialPeer(echoPort)
  assert.ok(conn, 'peer.conn set synchronously, like net.connect')
  await new Promise((resolve, reject) => {
    const t = setTimeout(() => reject(new Error('timed out')), 5000)
    conn.once('error', e => { clearTimeout(t); reject(e) })
    conn.once('connect', () => {
      conn.write('BitTorrent handshake bytes')
      conn.once('data', d => {
        clearTimeout(t)
        assert.equal(d.toString(), 'BitTorrent handshake bytes', 'round-tripped through the proxy')
        conn.destroy(); resolve()
      })
    })
  })
  assert.equal(socks.stats.connects.length, before + 1, 'proxy logged exactly one CONNECT')
  assert.ok(socks.stats.connects.at(-1).startsWith(`127.0.0.1:${echoPort}`))
})

await check('fetch (web seeds, http trackers) is routed through the proxy', async () => {
  const before = socks.stats.connects.length
  const res = await fetch(`http://127.0.0.1:${webPort}/announce?info_hash=abc`)
  assert.equal(await res.text(), 'announce-ok:/announce?info_hash=abc')
  assert.ok(socks.stats.connects.length > before,
    'proxy saw the announce; saw ' + JSON.stringify(socks.stats.connects.slice(before)))
})

await check('hostnames are resolved at the proxy, not locally (no DNS leak)', async () => {
  const before = socks.stats.connects.length
  await fetch('http://tracker.invalid/announce').catch(() => {})
  const seen = socks.stats.connects.slice(before)
  assert.ok(seen.some(c => c.startsWith('tracker.invalid:80') && c.includes('(name)')),
    'proxy received the hostname, saw: ' + JSON.stringify(seen))
})

await check('udp:// trackers are stripped from announce lists', async () => {
  const stub = {
    announce: ['udp://tracker.example:1337/announce', 'https://tracker.example/announce',
               'UDP://SHOUTY.example:80', 'wss://tracker.example'],
    discovery: null, destroyed: true
  }
  Torrent.prototype._startDiscovery.call(stub)
  assert.deepEqual(stub.announce, ['https://tracker.example/announce', 'wss://tracker.example'])
})

await check('announce lists are untouched when no proxy is set', async () => {
  useEgress(null)
  const stub = { announce: ['udp://tracker.example:1337/announce'], discovery: null, destroyed: true }
  Torrent.prototype._startDiscovery.call(stub)
  assert.deepEqual(stub.announce, ['udp://tracker.example:1337/announce'])
  useEgress(cfg)
})

await check('turning the proxy off restores direct fetch', async () => {
  useEgress(null)
  const before = socks.stats.connects.length
  const res = await fetch(`http://127.0.0.1:${webPort}/direct`)
  assert.equal(await res.text(), 'announce-ok:/direct')
  assert.equal(socks.stats.connects.length, before, 'proxy saw nothing')
  useEgress(cfg)
})

await check('a dead proxy fails the peer dial instead of falling back to direct', async () => {
  useEgress({ proxy: { host: '127.0.0.1', port: 1, type: 5 }, bind: null })
  const conn = dialPeer(echoPort)
  await new Promise((resolve, reject) => {
    const t = setTimeout(() => reject(new Error('neither connected nor errored')), 6000)
    conn.once('error', () => { clearTimeout(t); resolve() })
    conn.once('connect', () => { clearTimeout(t); reject(new Error('LEAKED: connected directly')) })
  })
  useEgress(cfg)
})

await check('a dead proxy fails fetch instead of falling back to direct', async () => {
  useEgress({ proxy: { host: '127.0.0.1', port: 1, type: 5 }, bind: null })
  await assert.rejects(fetch(`http://127.0.0.1:${webPort}/leak`))
  useEgress(cfg)
})

await check('username/password auth was performed on every proxied connection', async () => {
  assert.equal(socks.stats.auths, socks.stats.connects.length,
    `auths=${socks.stats.auths} connects=${socks.stats.connects.length}`)
})


// ---- interface binding -----------------------------------------------------

await check('bound peer dial leaves from the bound address', async () => {
  if (!nic) return console.log('       (skipped: no interface)')
  useEgress({ proxy: null, bind: nic.name })
  const before = seenFrom.length
  const conn = dialPeer(echoPort, nic.address)
  await new Promise((resolve, reject) => {
    const t = setTimeout(() => reject(new Error('timed out')), 5000)
    conn.once('error', e => { clearTimeout(t); reject(e) })
    conn.once('connect', () => { clearTimeout(t); conn.destroy(); resolve() })
  })
  await waitFor(() => seenFrom.length > before, 'the echo server to accept')
  assert.equal(seenFrom.at(-1), nic.address,
    `server saw ${seenFrom.at(-1)}, expected the bound ${nic.address}`)
  useEgress(cfg)
})

await check('bound fetch leaves from the bound address', async () => {
  if (!nic) return console.log('       (skipped: no interface)')
  useEgress({ proxy: null, bind: nic.name })
  const res = await fetch(`http://${nic.address}:${webPort}/announce`)
  assert.equal(await res.text(), 'announce-ok:/announce')
  assert.equal(webSeenFrom.at(-1), nic.address)
  useEgress(cfg)
})

await check('proxy + bind together: the hop to the proxy is the bound one', async () => {
  if (!nic) return console.log('       (skipped: no interface)')
  const before = socks.stats.from.length
  useEgress({ proxy: cfg.proxy, bind: nic.name })
  const conn = dialPeer(echoPort)
  await new Promise((resolve, reject) => {
    const t = setTimeout(() => reject(new Error('timed out')), 5000)
    conn.once('error', e => { clearTimeout(t); reject(e) })
    conn.once('connect', () => { clearTimeout(t); conn.destroy(); resolve() })
  })
  assert.ok(socks.stats.from.length > before, 'proxy saw the connection')
  assert.equal(socks.stats.from.at(-1), nic.address,
    `proxy saw us as ${socks.stats.from.at(-1)}, expected ${nic.address}`)
  useEgress(cfg)
})

await check('a vanished interface stops the dial rather than re-routing it', async () => {
  useEgress({ proxy: null, bind: 'utun-does-not-exist' })
  const before = seenFrom.length
  const conn = dialPeer(echoPort)
  const err = await new Promise((resolve, reject) => {
    const t = setTimeout(() => reject(new Error('neither connected nor errored')), 5000)
    conn.once('error', e => { clearTimeout(t); resolve(e) })
    conn.once('connect', () => { clearTimeout(t); reject(new Error('LEAKED: connected anyway')) })
  })
  assert.equal(err.code, 'EEGRESSDOWN', 'failed for the right reason: ' + err.message)
  assert.equal(seenFrom.length, before, 'nothing reached the server')
  useEgress(cfg)
})

await check('a vanished interface stops fetch rather than re-routing it', async () => {
  useEgress({ proxy: null, bind: 'utun-does-not-exist' })
  const before = webSeenFrom.length
  await assert.rejects(fetch(`http://127.0.0.1:${webPort}/leak`))
  assert.equal(webSeenFrom.length, before, 'nothing reached the server')
  useEgress(cfg)
})

await check('DHT.listen is given the bound address, and left alone otherwise', async () => {
  // The real listen() forwards to this._rpc.bind, so a stub records what it got.
  const forwarded = []
  const stub = { _rpc: { bind: (...args) => forwarded.push(args) } }
  if (nic) {
    useEgress({ proxy: null, bind: nic.name })
    DHT.prototype.listen.call(stub, 6881)
    assert.deepEqual(forwarded.at(-1), [6881, nic.address], 'bound DHT gets an address')
  }
  useEgress(cfg)                       // proxy set -> DHT is off entirely
  DHT.prototype.listen.call(stub, 6881)
  assert.deepEqual(forwarded.at(-1), [6881], 'no address forced under a proxy')
  useEgress(null)
  DHT.prototype.listen.call(stub, 6881)
  assert.deepEqual(forwarded.at(-1), [6881], 'untouched with no policy')
  useEgress(cfg)
})

await check('closeInbound shuts the listeners and tolerates their absence', async () => {
  const srv = net.createServer()
  await new Promise(r => srv.listen(0, r))
  const fake = { _connPool: { tcpServer: srv, utpServer: null } }
  assert.equal(closeInbound(fake), true)
  await new Promise(r => srv.on('close', r))
  assert.equal(closeInbound({}), false)
  assert.equal(closeInbound(null), false)
})

const failed = results.filter(r => r === 'FAIL').length
console.log(`\n${results.length - failed} passed, ${failed} failed`)
echo.close(); web.close(); socks.server.close()
process.exit(failed ? 1 : 0)
