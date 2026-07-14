//! Query the supervisor's output-presentation state without attaching a client.

use super::meta;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

/// Whether this PTY currently has an attached output client. A failed probe is
/// deliberately conservative: callers should treat unavailable PTY output as
/// not visible.
pub(crate) fn output_is_visible(id_or_path: &str) -> bool {
    let path = meta::resolve_socket(id_or_path);
    let Ok(mut stream) = UnixStream::connect(path) else {
        return false;
    };
    if stream.write_all(b"OUTPUT_MODE\n").is_err() || stream.flush().is_err() {
        return false;
    }
    let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
    let mut response = String::new();
    BufReader::new(stream)
        .read_line(&mut response)
        .is_ok_and(|read| read > 0 && response.trim() == "headed")
}

#[cfg(test)]
mod tests {
    use super::output_is_visible;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;

    #[test]
    fn output_visibility_reads_the_supervisor_mode() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("pty.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let worker = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream);
            let mut command = String::new();
            reader.read_line(&mut command).unwrap();
            assert_eq!(command, "OUTPUT_MODE\n");
            reader.get_mut().write_all(b"headed\n").unwrap();
        });

        assert!(output_is_visible(socket.to_str().unwrap()));
        worker.join().unwrap();
    }
}
