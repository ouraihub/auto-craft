//! Local static preview listener and retained runtime ownership.
use super::*;
/// Receives the URL of the currently served build without coupling rebuild to UI.
/// Receives and owns the active preview server without coupling rebuild to UI.
pub trait PreviewSink: Send + Sync {
    fn publish(&self, url: &str);
    fn retain(&self, server: PreviewServer);
}

/// A running static preview retained by a runtime owner.
pub struct PreviewServer(StaticServer);
impl PreviewServer {
    pub(super) fn start(root: &Path) -> Result<Self> {
        StaticServer::start(root).map(Self)
    }
    pub(super) fn url(&self) -> String {
        self.0.url()
    }
}
impl fmt::Debug for PreviewServer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreviewServer")
            .field("addr", &self.0.addr)
            .finish()
    }
}
impl fmt::Debug for PreviewSlot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreviewSlot")
            .field("url", &self.url())
            .finish()
    }
}
/// Runtime-owned preview slot; replacing a build shuts down its old listener.
#[derive(Default)]
pub struct PreviewSlot {
    server: Mutex<Option<PreviewServer>>,
    latest: Mutex<Option<String>>,
}
impl PreviewSlot {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn url(&self) -> Option<String> {
        self.server
            .lock()
            .ok()
            .and_then(|g| g.as_ref().map(|s| s.0.url()))
    }
    pub fn latest(&self) -> Option<String> {
        self.latest.lock().ok().and_then(|g| g.clone())
    }
    pub fn shutdown(&self) {
        if let Ok(mut g) = self.server.lock() {
            *g = None;
        }
    }
}
impl PreviewSink for PreviewSlot {
    fn publish(&self, url: &str) {
        if let Ok(mut latest) = self.latest.lock() {
            *latest = Some(url.to_owned());
        }
    }
    fn retain(&self, server: PreviewServer) {
        if let Ok(mut g) = self.server.lock() {
            *g = Some(server);
        }
    }
}
#[derive(Debug)]
struct StaticServer {
    addr: std::net::SocketAddr,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}
impl StaticServer {
    fn start(root: &Path) -> Result<Self> {
        let root = fs::canonicalize(root).map_err(|_| Error::not_found("preview folder"))?;
        if !root.is_dir() {
            return Err(Error::invalid("preview path is not a folder"));
        }
        let listener = TcpListener::bind((IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
            .map_err(|_| Error::external("preview server could not start"))?;
        listener
            .set_nonblocking(true)
            .map_err(|_| Error::external("preview server setup failed"))?;
        let addr = listener
            .local_addr()
            .map_err(|_| Error::external("preview server address unavailable"))?;
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let base = root.clone();
        let join = thread::spawn(move || {
            while !stopped.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => serve_request(&base, &mut stream),
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10))
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            addr,
            stop,
            join: Some(join),
        })
    }
    fn url(&self) -> String {
        format!("http://127.0.0.1:{}/", self.addr.port())
    }
}
impl Drop for StaticServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}
fn serve_request(root: &Path, stream: &mut std::net::TcpStream) {
    use std::io::Read;
    if stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .is_err()
        || stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .is_err()
    {
        return;
    }
    let mut buf = [0_u8; 8192];
    let mut used = 0;
    loop {
        let Ok(n) = stream.read(&mut buf[used..]) else {
            return;
        };
        if n == 0 {
            return;
        }
        used += n;
        if buf[..used].windows(4).any(|part| part == b"\r\n\r\n") {
            break;
        }
        if used == buf.len() {
            respond(stream, 431, "text/plain", b"headers too large");
            return;
        }
    }
    let request = String::from_utf8_lossy(&buf[..used]);
    let Some(path) = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
    else {
        return;
    };
    let Some(decoded) = decode_url_path(path.split('?').next().unwrap_or("/")) else {
        respond(stream, 400, "text/plain", b"bad path");
        return;
    };
    let rel = Path::new(decoded.trim_start_matches('/'));
    if rel.components().any(|c| {
        matches!(
            c,
            std::path::Component::ParentDir
                | std::path::Component::RootDir
                | std::path::Component::Prefix(_)
        )
    }) {
        respond(stream, 403, "text/plain", b"forbidden");
        return;
    }
    let target = root.join(rel);
    let target = if target.is_dir() {
        target.join("index.html")
    } else {
        target
    };
    let Ok(real) = fs::canonicalize(&target) else {
        respond(stream, 404, "text/plain", b"not found");
        return;
    };
    if !real.starts_with(root) {
        respond(stream, 403, "text/plain", b"forbidden");
        return;
    }
    let Ok(body) = fs::read(&real) else {
        respond(stream, 404, "text/plain", b"not found");
        return;
    };
    let mime = mime_for(&real);
    respond(stream, 200, mime, &body);
}
fn decode_url_path(path: &str) -> Option<String> {
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return None;
            }
            let hi = (bytes[i + 1] as char).to_digit(16)?;
            let lo = (bytes[i + 2] as char).to_digit(16)?;
            out.push((hi * 16 + lo) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}
fn mime_for(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" => "application/json",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}
fn respond(stream: &mut std::net::TcpStream, code: u16, mime: &str, body: &[u8]) {
    use std::io::Write;
    let _ = write!(
        stream,
        "HTTP/1.1 {code}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(body);
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(server: &StaticServer, path: &str) -> String {
        use std::io::{Read, Write};
        let mut socket = std::net::TcpStream::connect(server.addr).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        write!(socket, "GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
        let mut response = String::new();
        socket.read_to_string(&mut response).unwrap();
        response
    }

    #[test]
    fn preview_serves_encoded_assets_but_rejects_encoded_traversal() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("hello world.css"), "body { color: red; }").unwrap();
        let server = StaticServer::start(root.path()).unwrap();
        let response = request(&server, "/hello%20world.css");
        assert!(response.starts_with("HTTP/1.1 200\r\n"));
        assert!(response.contains("Content-Type: text/css; charset=utf-8\r\n"));
        assert!(response.ends_with("body { color: red; }"));
        assert!(request(&server, "/%2e%2e/private").starts_with("HTTP/1.1 403\r\n"));
        assert!(request(&server, "/%FF").starts_with("HTTP/1.1 400\r\n"));
    }

    #[test]
    fn idle_preview_client_cannot_block_shutdown() {
        let root = tempfile::tempdir().unwrap();
        let server = StaticServer::start(root.path()).unwrap();
        let client = std::net::TcpStream::connect(server.addr).unwrap();
        let (done, result) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            drop(server);
            done.send(()).unwrap();
        });
        result
            .recv_timeout(Duration::from_secs(3))
            .expect("preview shutdown must not wait for client input");
        drop(client);
    }
}
