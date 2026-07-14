use super::{trace, trace_bytes};
use anyhow::Result;
use std::collections::VecDeque;
use std::io::{BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};

type Client = Arc<Mutex<UnixStream>>;
pub(super) type Clients = Arc<Mutex<Vec<Client>>>;

pub(super) fn output_mode(clients: &Clients) -> &'static str {
    if clients.lock().unwrap().is_empty() {
        "headless"
    } else {
        "headed"
    }
}

pub(super) fn attach_client(
    mut reader: BufReader<UnixStream>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    clients: &Clients,
    backlog: &Arc<Mutex<VecDeque<u8>>>,
) -> Result<()> {
    let mut output = reader.get_ref().try_clone()?;
    let remembered = backlog.lock().unwrap().iter().copied().collect::<Vec<_>>();
    if !remembered.is_empty() {
        let _ = output.write_all(&remembered);
    }
    let client = Arc::new(Mutex::new(output));
    clients.lock().unwrap().push(client.clone());
    let clients = clients.clone();
    std::thread::spawn(move || {
        let mut buf = [0_u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => {
                    trace("supervisor attach eof");
                    break;
                }
                Ok(n) => {
                    trace_bytes("supervisor attach", &buf[..n]);
                    let mut writer = writer.lock().unwrap();
                    let result = writer.write_all(&buf[..n]).and_then(|_| writer.flush());
                    if result.is_err() {
                        trace("supervisor attach write error");
                        break;
                    }
                }
                Err(_) => {
                    trace("supervisor attach read error");
                    break;
                }
            }
        }
        clients
            .lock()
            .unwrap()
            .retain(|entry| !Arc::ptr_eq(entry, &client));
    });
    Ok(())
}

pub(super) fn fanout(clients: &Clients, bytes: &[u8]) {
    let mut clients = clients.lock().unwrap();
    clients.retain(|client| {
        let Ok(mut stream) = client.lock() else {
            return false;
        };
        stream.write_all(bytes).and_then(|_| stream.flush()).is_ok()
    });
}
