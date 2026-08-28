//! Owner-only secret file writes (Bitcoin Core cookie / SSH key discipline).
//!
//! Unix: create the file `0600` from the first write (no chmod-after window).
//! Windows: write then restrict the DACL to the current user when `icacls` is available.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

/// Write `contents` atomically (temp + rename) with owner-only permissions.
/// Overwrites an existing file.
pub fn write_secret_file(path: &Path, contents: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let tmp = path.with_extension("tmp-secret");
    {
        let mut opts = OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp)?;
        f.write_all(contents)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    restrict_secret_file(path)?;
    Ok(())
}

/// Same as [`write_secret_file`] but refuses to replace an existing path.
pub fn write_secret_file_no_clobber(path: &Path, contents: &[u8]) -> io::Result<()> {
    if path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("refusing to overwrite secret {}", path.display()),
        ));
    }
    write_secret_file(path, contents)
}

/// Tighten permissions on an existing secret (idempotent).
pub fn restrict_secret_file(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path)?.permissions();
        perms.set_mode(0o600);
        fs::set_permissions(path, perms)?;
    }
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("icacls")
            .arg(path)
            .arg("/inheritance:r")
            .arg("/grant:r")
            .arg(format!("{}:(R,W)", whoami_windows()))
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    Ok(())
}

#[cfg(windows)]
fn whoami_windows() -> String {
    std::env::var("USERNAME").unwrap_or_else(|_| "%USERNAME%".into())
}

/// 32-byte OS CSPRNG secret, lowercase hex (256-bit cookie / AI token).
pub fn mint_secret_hex() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    let hex = hex::encode(bytes);
    bytes.iter_mut().for_each(|b| *b = 0);
    hex
}
