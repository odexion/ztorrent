//! A SOCKS5 server for the egress tests, counting what it is asked to do --
//! the Rust twin of scripts/socks-server.mjs.

use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;

#[derive(Default, Debug)]
pub struct Stats {
    pub connects: Vec<String>,
    pub auths: usize,
}

pub struct Socks {
    pub port: u16,
    pub stats: Arc<Mutex<Stats>>,
}

pub fn start_socks(user: &'static str, pass: &'static str) -> Socks {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let stats = Arc::new(Mutex::new(Stats::default()));
    let s = stats.clone();
    thread::spawn(move || {
        for conn in listener.incoming().flatten() {
            let s = s.clone();
            thread::spawn(move || {
                let _ = serve(conn, user, pass, &s);
            });
        }
    });
    Socks { port, stats }
}

fn serve(mut c: TcpStream, user: &str, pass: &str, stats: &Mutex<Stats>) -> std::io::Result<()> {
    let mut head = [0u8; 2];
    c.read_exact(&mut head)?;
    let mut methods = vec![0u8; head[1] as usize];
    c.read_exact(&mut methods)?;
    if methods.contains(&2) {
        c.write_all(&[5, 2])?;
        let mut v = [0u8; 2];
        c.read_exact(&mut v)?;
        let mut u = vec![0u8; v[1] as usize];
        c.read_exact(&mut u)?;
        let mut plen = [0u8; 1];
        c.read_exact(&mut plen)?;
        let mut p = vec![0u8; plen[0] as usize];
        c.read_exact(&mut p)?;
        let ok = u == user.as_bytes() && p == pass.as_bytes();
        c.write_all(&[1, if ok { 0 } else { 1 }])?;
        if !ok {
            return Ok(());
        }
        stats.lock().unwrap().auths += 1;
    } else {
        c.write_all(&[5, 0])?;
    }
    let mut req = [0u8; 4];
    c.read_exact(&mut req)?;
    let host = match req[3] {
        1 => {
            let mut a = [0u8; 4];
            c.read_exact(&mut a)?;
            format!("{}.{}.{}.{}", a[0], a[1], a[2], a[3])
        }
        3 => {
            let mut l = [0u8; 1];
            c.read_exact(&mut l)?;
            let mut n = vec![0u8; l[0] as usize];
            c.read_exact(&mut n)?;
            format!("{} (name)", String::from_utf8_lossy(&n))
        }
        4 => {
            let mut a = [0u8; 16];
            c.read_exact(&mut a)?;
            std::net::Ipv6Addr::from(a).to_string()
        }
        _ => return Ok(()),
    };
    let mut pb = [0u8; 2];
    c.read_exact(&mut pb)?;
    let port = u16::from_be_bytes(pb);
    stats.lock().unwrap().connects.push(format!("{host}:{port}"));
    let target_host = host.trim_end_matches(" (name)");
    let upstream = match TcpStream::connect((target_host, port)) {
        Ok(u) => u,
        Err(_) => {
            c.write_all(&[5, 5, 0, 1, 0, 0, 0, 0, 0, 0])?;
            return Ok(());
        }
    };
    c.write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0])?;
    let (mut a, mut b) = (c.try_clone()?, upstream.try_clone()?);
    let (mut c2, mut u2) = (c, upstream);
    thread::spawn(move || {
        let _ = std::io::copy(&mut a, &mut u2);
        let _ = u2.shutdown(Shutdown::Write);
    });
    let _ = std::io::copy(&mut b, &mut c2);
    let _ = c2.shutdown(Shutdown::Write);
    Ok(())
}
