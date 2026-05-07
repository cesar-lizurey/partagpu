//! Workspace materialization on the host : decode the payload sent by the
//! requester (base64, optionally gzipped), write each file under a tmp dir
//! that bwrap will bind-mount as `/workspace` inside the sandbox.
//!
//! Also exposes [`compress_workspace`] used by the dispatcher to gzip the
//! payload before encryption (saves bytes on the wire ; AES-GCM ciphertext
//! is incompressible so we have to gzip *before*).

use std::path::PathBuf;

use super::WorkspaceFile;

/// Hard cap on the cumulative size of all files in a single workspace
/// payload (decompressed). Protects against OOM from a malicious peer.
pub(super) const MAX_WORKSPACE_BYTES: u64 = 16 * 1024 * 1024; // 16 MB total

/// A scratch directory on the host that's bind-mounted as /workspace inside
/// the sandbox. Cleaned up when this struct is dropped.
pub(super) struct TempWorkspace {
    pub(super) path: PathBuf,
}

impl Drop for TempWorkspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

pub(super) fn prepare_workspace(files: &[WorkspaceFile]) -> Result<TempWorkspace, String> {
    use std::os::unix::fs::PermissionsExt;

    // We always create the workspace dir under /tmp (or whatever
    // std::env::temp_dir() returns). The app runs as the regular user (e.g.
    // `cesar`), the sandbox runs as the `partagpu` UID, so the directory
    // needs to be world-writable for the sandbox to create output files.
    // /var/lib/partagpu would be more elegant but its mode 700 + ownership
    // by the partagpu user blocks creation from the app process.
    let base = std::env::temp_dir();
    let dir = base.join(format!("partagpu-task-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("création workspace : {e}"))?;

    // 0o777: anyone (including the partagpu UID inside the sandbox) can
    // create / delete files in this dir. The dir itself is in /tmp so it
    // benefits from /tmp's sticky bit when applicable.
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777))
        .map_err(|e| format!("chmod workspace : {e}"))?;

    let mut total_bytes: u64 = 0;
    for f in files {
        let safe = sanitize_relative_path(&f.path)?;
        let full = dir.join(&safe);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", f.path))?;
            // Sub-dirs created here also need to be writable by the sandbox UID.
            let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o777));
        }
        let raw = data_encoding::BASE64
            .decode(f.content_b64.as_bytes())
            .map_err(|e| format!("base64 invalide pour {}: {e}", f.path))?;
        // Decompress if the sender flagged the bytes as gzipped. Older
        // clients (legacy) send raw bytes without the compression field.
        let bytes = match f.compression.as_deref() {
            Some("gzip") => {
                use std::io::Read;
                let mut decoder = flate2::read::GzDecoder::new(raw.as_slice());
                let mut decompressed = Vec::with_capacity(raw.len() * 2);
                decoder
                    .read_to_end(&mut decompressed)
                    .map_err(|e| format!("gunzip invalide pour {}: {e}", f.path))?;
                decompressed
            }
            None | Some("none") => raw,
            Some(other) => {
                return Err(format!("compression inconnue pour {}: {other}", f.path));
            }
        };
        total_bytes = total_bytes.saturating_add(bytes.len() as u64);
        if total_bytes > MAX_WORKSPACE_BYTES {
            return Err(format!(
                "workspace dépasse la limite de {} octets",
                MAX_WORKSPACE_BYTES
            ));
        }
        std::fs::write(&full, &bytes).map_err(|e| format!("écriture {}: {e}", f.path))?;
        // Make the file world-readable so the sandbox UID can read it,
        // and writable so a training script can overwrite a config in place
        // if needed.
        let _ = std::fs::set_permissions(&full, std::fs::Permissions::from_mode(0o666));
    }

    Ok(TempWorkspace { path: dir })
}

/// Compress a list of WorkspaceFile in-place : their `content_b64` is
/// re-encoded to hold gzipped bytes (`compression = Some("gzip")`).
/// Used by the dispatcher to shrink the network payload sent to the peer.
/// Files are compressed individually (per-file gzip) so the peer can
/// stream-decode each.
pub fn compress_workspace(files: &mut [WorkspaceFile]) -> Result<(), String> {
    use flate2::{write::GzEncoder, Compression};
    use std::io::Write;
    for f in files {
        if matches!(f.compression.as_deref(), Some("gzip")) {
            continue; // already compressed, idempotent
        }
        let raw = data_encoding::BASE64
            .decode(f.content_b64.as_bytes())
            .map_err(|e| format!("base64 invalide pour {}: {e}", f.path))?;
        let mut enc = GzEncoder::new(Vec::with_capacity(raw.len() / 2), Compression::default());
        enc.write_all(&raw)
            .map_err(|e| format!("gzip écriture {}: {e}", f.path))?;
        let gz = enc
            .finish()
            .map_err(|e| format!("gzip finish {}: {e}", f.path))?;
        f.content_b64 = data_encoding::BASE64.encode(&gz);
        f.compression = Some("gzip".to_string());
    }
    Ok(())
}

/// Validate a workspace-relative path: no absolute, no `..`, no NUL.
fn sanitize_relative_path(p: &str) -> Result<PathBuf, String> {
    if p.is_empty() {
        return Err("chemin workspace vide".into());
    }
    if p.contains('\0') {
        return Err("chemin workspace contient un NUL".into());
    }
    let path = PathBuf::from(p);
    if path.is_absolute() {
        return Err(format!("chemin workspace doit être relatif : {p}"));
    }
    for comp in path.components() {
        use std::path::Component::*;
        match comp {
            Normal(_) | CurDir => {}
            ParentDir => return Err(format!("chemin workspace contient '..' : {p}")),
            RootDir | Prefix(_) => return Err(format!("chemin workspace invalide : {p}")),
        }
    }
    Ok(path)
}
