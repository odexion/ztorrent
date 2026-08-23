// Minimal SOCKS5 server: no-auth + username/password, CONNECT only.
// Counts connections so a test can prove traffic actually went through it.
import net from 'node:net'

export function startSocks ({ user, pass } = {}) {
  const stats = { connects: [], auths: 0, from: [] }
  const server = net.createServer(sock => {
    stats.from.push(sock.remoteAddress.replace(/^::ffff:/, ''))
    sock.once('data', greet => {
      const nMethods = greet[1]
      const methods = [...greet.subarray(2, 2 + nMethods)]
      const want = user ? 0x02 : 0x00
      if (!methods.includes(want)) return sock.end(Buffer.from([0x05, 0xff]))
      sock.write(Buffer.from([0x05, want]))
      if (want === 0x02) {
        sock.once('data', auth => {
          const ulen = auth[1]
          const u = auth.subarray(2, 2 + ulen).toString()
          const plen = auth[2 + ulen]
          const p = auth.subarray(3 + ulen, 3 + ulen + plen).toString()
          const ok = u === user && p === pass
          stats.auths += ok ? 1 : 0
          sock.write(Buffer.from([0x01, ok ? 0x00 : 0x01]))
          if (ok) sock.once('data', req => onRequest(sock, req, stats))
          else sock.end()
        })
      } else {
        sock.once('data', req => onRequest(sock, req, stats))
      }
    })
    sock.on('error', () => {})
  })
  return { server, stats, listen: () => new Promise(r => server.listen(0, '127.0.0.1', () => r(server.address().port))) }
}

function onRequest (sock, req, stats) {
  const atyp = req[3]
  let host, offset
  if (atyp === 0x01) { host = [...req.subarray(4, 8)].join('.'); offset = 8 }
  else if (atyp === 0x03) { const l = req[4]; host = req.subarray(5, 5 + l).toString(); offset = 5 + l }
  else return sock.end()
  const port = req.readUInt16BE(offset)
  stats.connects.push(`${host}:${port}` + (atyp === 0x03 ? ' (name)' : ''))

  const up = net.connect(port, atyp === 0x03 ? host : host, () => {
    const rep = Buffer.alloc(10)
    rep[0] = 0x05; rep[1] = 0x00; rep[3] = 0x01
    sock.write(rep)
    sock.pipe(up); up.pipe(sock)
  })
  up.on('error', () => { sock.end(Buffer.from([0x05, 0x01, 0x00, 0x01, 0, 0, 0, 0, 0, 0])) })
}
