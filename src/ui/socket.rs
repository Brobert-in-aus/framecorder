//! A control socket, so other programs on the headset (a wrist panel, a
//! script) can clip and record without the dashboard tab open.
//!
//! `$XDG_RUNTIME_DIR/framecorder/control.sock`, only for this user. Connect,
//! send one line, read one line of JSON back:
//!
//! - `status`: what's going on
//! - `clip`: save the clip buffer, the same as the tab's clip button
//! - `record`: start a recording, if one isn't going
//! - `stop`: stop the recording, if one is
//!
//! Every reply is the status after the command, plus `"ok"` and, when it
//! couldn't, `"error"`. Commands run on the tab's own loop, through the same
//! code as its buttons, so the tab, the toasts and the buzz all keep up.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use anyhow::{Context, Result};

/// How long a client waits for the tab's loop to get to its command.
const REPLY_WAIT: Duration = Duration::from_secs(3);

/// A command from a client, and where its reply goes.
pub struct Request {
    pub command: String,
    pub reply: Sender<String>,
}

pub fn path() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    dir.join("framecorder").join("control.sock")
}

/// Starts listening; requests arrive on the returned channel for the tab's
/// loop to answer.
pub fn listen() -> Result<Receiver<Request>> {
    let path = path();
    let dir = path.parent().context("socket path has no folder")?;
    std::fs::create_dir_all(dir)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    // Only one tab runs (only_one), so anything there is left over.
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).with_context(|| format!("binding {}", path.display()))?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new().name("control-socket".into()).spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let _ = stream.set_read_timeout(Some(REPLY_WAIT));
            let mut line = String::new();
            if BufReader::new(&stream).read_line(&mut line).is_err() {
                continue;
            }
            let (reply_tx, reply_rx) = mpsc::channel();
            if tx.send(Request { command: line.trim().to_string(), reply: reply_tx }).is_err() {
                break; // the tab's gone
            }
            let reply = reply_rx
                .recv_timeout(REPLY_WAIT)
                .unwrap_or_else(|_| r#"{"ok":false,"error":"framecorder is busy"}"#.to_string());
            let _ = writeln!(stream, "{reply}");
        }
    })?;
    log::info!("control socket at {}", path.display());
    Ok(rx)
}
