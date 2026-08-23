import net from 'node:net'
import os from 'node:os'
import { Duplex } from 'node:stream'
import { SocksClient } from 'socks'
import { Agent, buildConnector, setGlobalDispatcher, getGlobalDispatcher } from 'undici'
import Torrent from 'webtorrent/lib/torrent.js'
import { Client as DHT } from 'bittorrent-dht'

/**
 * Everything that decides where our traffic leaves from, and refuses to let it
 * leave any other way.
 *
 * Two policies, usable together:
 *
 *   proxy   SOCKS5 for tracker announces, web-seed fetches and outgoing peer
 *           connections, with names resolved at the proxy so no DNS leaks.
 *   bind    every outbound socket pinned to one interface's address, so when
 *           that interface goes away -- a VPN dropping -- connections fail
 *           instead of quietly re-routing through the physical NIC.
 *
 * The rule throughout is fail-closed. A policy that covers announces but leaks
 * peer connections is worse than none at all: it costs speed and hides nothing,
 * because the address the swarm records is the one the peers see. So anything
 * that cannot be routed is switched off rather than sent direct:
 *
 *   under a proxy   DHT, LSD, uTP and UPnP are disabled (all UDP or inbound),
 *                   udp:// trackers are filtered out of every announce list,
 *                   and the inbound listeners are closed
 *   under a bind    LSD, uTP and UPnP are disabled and udp:// trackers filtered
 *                   for the same reason -- but DHT stays, bound to the same
 *                   address as everything else
 *
 * The bound address is re-read per dial, so a VPN that comes back on a new
 * address resumes on its own, and one that goes away stops everything.
 */

const CONNECT_TIMEOUT = 20_000

// Captured before anything patches it. The peer-dial patch below replaces
// net.connect while _drain runs, and the dialler it installs must reach the
// real one -- calling the patched version would recurse forever.
const realConnect = net.connect.bind(net)

/** Every non-internal IPv4 interface, for the preferences dropdown. */
export function listInterfaces () {
  const out = []
  for (const [name, addrs] of Object.entries(os.networkInterfaces())) {
    for (const a of addrs || []) {
      if (a.family === 'IPv4' && !a.internal) out.push({ name, address: a.address })
    }
  }
  return out
}

/**
 * Current address of a bound interface, re-read rather than remembered: a VPN
 * that reconnects usually comes back on a different one. Null means the
 * interface is gone, which is the signal to stop dialling entirely.
 */
let addrCache = { at: 0, name: null, address: null }
function bindAddress (name) {
  if (!name) return null
  const now = Date.now()
  if (addrCache.name === name && now - addrCache.at < 1000) return addrCache.address
  const found = listInterfaces().find(i => i.name === name)
  addrCache = { at: now, name, address: found ? found.address : null }
  return addrCache.address
}

/** Reads the proxy out of settings, or null when it is off or incomplete. */
export function proxyConfig (s) {
  if (!s?.proxyEnabled) return null
  const host = String(s.proxyHost || '').trim()
  const port = Number(s.proxyPort) || 0
  if (!host || port <= 0) return null
  const cfg = { host, port, type: 5 }
  if (s.proxyUsername) {
    cfg.userId = String(s.proxyUsername)
    cfg.password = String(s.proxyPassword || '')
  }
  return cfg
}

/** The whole egress policy: which proxy, which interface, or neither. */
export function egressPolicy (s) {
  const proxy = proxyConfig(s)
  const bind = s?.bindInterface ? String(s.bindInterface) : null
  return (proxy || bind) ? { proxy, bind } : null
}

/** True when two policies are the same, so a settings change can skip a restart. */
export function samePolicy (a, b) {
  if (!a || !b) return !a === !b
  if (a.bind !== b.bind) return false
  const [p, q] = [a.proxy, b.proxy]
  if (!p || !q) return !p === !q
  return p.host === q.host && p.port === q.port &&
         p.userId === q.userId && p.password === q.password
}

/** Thrown rather than dialling around a bind whose interface has vanished. */
class InterfaceGone extends Error {
  constructor (name) {
    super(`bound interface ${name} has no address -- refusing to connect`)
    this.code = 'EEGRESSDOWN'
  }
}

/**
 * Opens a TCP connection to `host:port` through the proxy.
 * Resolves the name at the proxy (ATYP 3), so no DNS lookup leaks locally.
 */
function socksConnect (proxy, host, port, localAddress) {
  const opts = {
    proxy,
    command: 'connect',
    destination: { host, port: Number(port) },
    timeout: CONNECT_TIMEOUT
  }
  // The hop we bind is the one to the proxy -- everything past it is the
  // proxy's own connection.
  if (localAddress) {
    opts.existing_socket = realConnect({
      host: proxy.host, port: proxy.port, localAddress
    })
  }
  return SocksClient.createConnection(opts).then(info => info.socket)
}

/** One outbound TCP connection under whatever policy is in force. */
function dial (policy, host, port) {
  const localAddress = policy.bind ? bindAddress(policy.bind) : null
  if (policy.bind && !localAddress) return Promise.reject(new InterfaceGone(policy.bind))
  if (policy.proxy) return socksConnect(policy.proxy, host, port, localAddress)
  return new Promise((resolve, reject) => {
    const sock = realConnect({ host, port: Number(port), localAddress })
    sock.once('connect', () => { sock.removeListener('error', reject); resolve(sock) })
    sock.once('error', reject)
  })
}

/**
 * A socket stand-in that can be handed back synchronously and wired up once
 * the SOCKS handshake finishes. WebTorrent's dialer expects `net.connect`'s
 * shape: an object returned immediately that emits 'connect' later.
 */
class DeferredSocket extends Duplex {
  constructor (dial) {
    super()
    this._sock = null
    this._queued = []
    dial.then(sock => this._attach(sock)).catch(err => this.destroy(err))
  }

  _attach (sock) {
    if (this.destroyed) return sock.destroy()
    this._sock = sock
    sock.on('data', chunk => { if (!this.push(chunk)) sock.pause() })
    sock.on('end', () => this.push(null))
    sock.on('error', err => this.destroy(err))
    sock.on('close', () => this.destroy())
    for (const [chunk, enc, cb] of this._queued) sock.write(chunk, enc, cb)
    this._queued = []
    this.emit('connect')
  }

  _read () { this._sock?.resume() }

  _write (chunk, enc, cb) {
    if (this._sock) this._sock.write(chunk, enc, cb)
    else this._queued.push([chunk, enc, cb])
  }

  _destroy (err, cb) {
    this._sock?.destroy()
    // Writes still waiting on a handshake that will never land.
    for (const [, , queuedCb] of this._queued) queuedCb?.(err || new Error('socket destroyed'))
    this._queued = []
    cb(err)
  }
}

/** An undici dispatcher that dials under the policy instead of directly. */
function makeDispatcher (policy) {
  const upgradeTLS = buildConnector({})
  return new Agent({
    connect (opts, callback) {
      const port = Number(opts.port) || (opts.protocol === 'https:' ? 443 : 80)
      dial(policy, opts.hostname, port).then(socket => {
        // buildConnector refuses a pre-made socket for plain HTTP, and for
        // HTTPS it is the thing that knows how to negotiate the certificate.
        if (opts.protocol === 'https:') upgradeTLS({ ...opts, httpSocket: socket }, callback)
        else callback(null, socket)
      }).catch(err => callback(err))
    }
  })
}

let installed = false
let active = null            // the policy every patched call site reads
let directDispatcher = null  // undici's own, kept so we can put it back

/**
 * Patches WebTorrent's outbound paths once, then leaves them keyed on `active`.
 * Every patch is a no-op while no policy is set, so this stays installed for
 * the life of the process and toggling costs nothing.
 */
function install () {
  if (installed) return
  installed = true

  // Peer dialling. _drain calls net.connect synchronously, so swapping it for
  // the duration of that call is enough -- nothing else can interleave, and no
  // other part of the app sees a patched net module.
  const drain = Torrent.prototype._drain
  Torrent.prototype._drain = function (...args) {
    if (!active) return drain.apply(this, args)
    const policy = active
    const real = net.connect
    net.connect = opts => new DeferredSocket(dial(policy, opts.host, opts.port))
    try {
      return drain.apply(this, args)
    } finally {
      net.connect = real
    }
  }

  // Announce lists, filtered at the last moment before a tracker client is
  // built from them. UDP announces are a bare dgram socket either way: there is
  // no SOCKS relay for them here, and an unbound socket ignores the bind.
  const startDiscovery = Torrent.prototype._startDiscovery
  Torrent.prototype._startDiscovery = function (...args) {
    if (active && Array.isArray(this.announce)) {
      this.announce = this.announce.filter(url => !/^udp:/i.test(String(url)))
    }
    return startDiscovery.apply(this, args)
  }

  // DHT survives a bind (unlike a proxy) as long as its socket is pinned to the
  // same address. WebTorrent calls listen(port) with no address of its own.
  const dhtListen = DHT.prototype.listen
  DHT.prototype.listen = function (...args) {
    const bind = active?.bind && !active.proxy ? bindAddress(active.bind) : null
    if (bind && typeof args[0] === 'number' && args.length === 1) {
      return dhtListen.call(this, args[0], bind)
    }
    return dhtListen.apply(this, args)
  }
}

/**
 * Closes the inbound listeners. Under a proxy nothing should reach us directly
 * at all, and a socket accepting on the real address is exactly the thing the
 * proxy exists to avoid.
 */
export function closeInbound (client) {
  const pool = client?._connPool
  if (!pool) return false
  pool.tcpServer?.close()
  pool.utpServer?.close()
  return true
}

/**
 * Turns a policy on or off process-wide. Returns a short description of what is
 * now in force, for the log.
 */
export function useEgress (policy) {
  install()
  active = policy
  if (policy) {
    directDispatcher = directDispatcher || getGlobalDispatcher()
    // Covers web-seed fetches and HTTP(S) tracker announces in one move --
    // both go through global fetch, and neither takes an agent of its own.
    setGlobalDispatcher(makeDispatcher(policy))
    const parts = []
    if (policy.proxy) {
      parts.push(`SOCKS5 ${policy.proxy.host}:${policy.proxy.port}` +
                 (policy.proxy.userId ? ' (authenticated)' : ''))
    }
    if (policy.bind) {
      const addr = bindAddress(policy.bind)
      parts.push(`bound to ${policy.bind}${addr ? ` (${addr})` : ' -- currently has no address'}`)
    }
    return parts.join(', ')
  }
  if (directDispatcher) setGlobalDispatcher(directDispatcher)
  return null
}

/** Client options that would leak around the policy, forced off while it is on. */
export function guardedOptions (policy) {
  if (!policy) return {}
  // LSD is a LAN broadcast and uTP is UDP: neither can be proxied, and neither
  // honours a bind. UPnP asks the router for a direct inbound path.
  const off = { lsd: false, utp: false, natUpnp: false, natPmp: false }
  // DHT is UDP too, but its socket can at least be pinned to the bound address.
  return policy.proxy ? { ...off, dht: false } : off
}
