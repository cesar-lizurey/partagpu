//! Small process / IO helpers used by the sandbox executor.

use std::io::Read;
use std::process::Command;
use std::sync::{Arc, Mutex};

/// Continuously read 4 KB chunks from `stream` and append them (as UTF-8) to
/// `buf`, capping the total length at `cap` bytes. Excess bytes after the cap
/// are dropped silently. Exits cleanly on EOF or read error.
pub(super) fn drain_stream<R: Read>(mut stream: R, buf: Arc<Mutex<String>>, cap: usize) {
    let mut chunk = [0u8; 4096];
    let mut leftover: Vec<u8> = Vec::new();
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                let mut data = if leftover.is_empty() {
                    chunk[..n].to_vec()
                } else {
                    let mut combined = std::mem::take(&mut leftover);
                    combined.extend_from_slice(&chunk[..n]);
                    combined
                };

                // Trim a possible split UTF-8 multibyte at the end of the
                // chunk so from_utf8_lossy doesn't insert a replacement char
                // mid-character. The leftover bytes carry over to next round.
                let valid_up_to = match std::str::from_utf8(&data) {
                    Ok(_) => data.len(),
                    Err(e) => e.valid_up_to(),
                };
                let tail = data.split_off(valid_up_to);
                leftover = tail;

                let s = match std::str::from_utf8(&data) {
                    Ok(s) => s,
                    Err(_) => continue, // shouldn't happen; defensively skip
                };

                let mut locked = match buf.lock() {
                    Ok(l) => l,
                    Err(_) => break,
                };
                if locked.len() >= cap {
                    continue;
                }
                let remaining = cap - locked.len();
                if s.len() <= remaining {
                    locked.push_str(s);
                } else {
                    // Find a char boundary at or before `remaining` so we
                    // never split a UTF-8 codepoint.
                    let mut idx = remaining;
                    while idx > 0 && !s.is_char_boundary(idx) {
                        idx -= 1;
                    }
                    locked.push_str(&s[..idx]);
                }
            }
            Err(_) => break,
        }
    }
}

pub(super) fn get_user_uid(user: &str) -> u32 {
    Command::new("id")
        .args(["-u", user])
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok())
        .unwrap_or(0)
}

pub(super) fn get_user_gid(user: &str) -> u32 {
    Command::new("id")
        .args(["-g", user])
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok())
        .unwrap_or(0)
}

pub(super) fn wait_with_timeout(
    child: &mut std::process::Child,
    timeout_secs: u64,
) -> Result<i32, String> {
    use std::time::{Duration, Instant};

    let deadline = Instant::now() + Duration::from_secs(timeout_secs);

    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status.code().unwrap_or(-1)),
            Ok(None) => {
                if Instant::now() > deadline {
                    return Err(format!(
                        "Tâche interrompue : dépassement du délai de {timeout_secs} secondes."
                    ));
                }
                std::thread::sleep(Duration::from_millis(250));
            }
            Err(e) => return Err(format!("Erreur d'attente : {e}")),
        }
    }
}
