use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Accepts TCP connections but never completes a WebSocket handshake or sends
/// a relay frame, deterministically wedging the NMP relay session.
pub(crate) struct WedgeRelay {
    pub(crate) url: String,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl WedgeRelay {
    pub(crate) fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind wedge relay");
        listener
            .set_nonblocking(true)
            .expect("set wedge relay nonblocking");
        let port = listener.local_addr().expect("wedge relay address").port();
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        let thread = std::thread::spawn(move || run(listener, thread_stop));
        Self {
            url: format!("ws://127.0.0.1:{port}"),
            stop,
            thread: Some(thread),
        }
    }
}

fn run(listener: TcpListener, stop: Arc<AtomicBool>) {
    let mut connections = Vec::<TcpStream>::new();
    while !stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, _)) => connections.push(stream),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            Err(_) => return,
        }
        for stream in &mut connections {
            let _ = stream.set_nonblocking(true);
            let _ = stream.read(&mut [0_u8; 1024]);
        }
    }
}

impl Drop for WedgeRelay {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
