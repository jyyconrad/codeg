//! Token store for GitHub accounts and chat-channel secrets.
//!
//! Primary storage is `tokens.json` under `CODEG_DATA_DIR` (mode 0600).
//! macOS Keychain items created by `keyring` are ACL-bound to the current
//! app signature, so replacing the `.app` (ad-hoc or a new package) prompts
//! for the login password on every install — including the git credential
//! helper, which is a second process of the same binary. File storage
//! survives replacement. Desktop builds still *read* leftover Keychain
//! items once and copy them into the file so existing tokens keep working.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[cfg(feature = "tauri-runtime")]
const SERVICE_NAME: &str = "codeg";

fn token_key(account_id: &str) -> String {
    format!("github-token:{}", account_id)
}

fn channel_token_key(channel_id: i32) -> String {
    format!("chat-channel:{}", channel_id)
}

fn default_data_dir() -> PathBuf {
    // Desktop identifier-derived folder vs server `codeg/`. Production
    // always pins `CODEG_DATA_DIR` at startup, so this is the no-env
    // fallback (tests, and a helper invoked without `--data-dir`).
    let name = if cfg!(feature = "tauri-runtime") {
        "app.codeg"
    } else {
        "codeg"
    };
    dirs::data_dir()
        .map(|d| d.join(name))
        .unwrap_or_else(|| PathBuf::from(".codeg-data"))
}

/// Namespace for secrets that are not tokens of an account or a channel. The
/// prefix keeps them from ever colliding with a `github-token:<id>` whose id
/// happens to look like a secret name.
fn secret_key(name: &str) -> String {
    format!("secret:{name}")
}

fn tokens_file_path() -> PathBuf {
    tokens_file_path_for(std::env::var("CODEG_DATA_DIR").ok().as_deref())
}

/// Resolve the on-disk `tokens.json` path given an explicit
/// `CODEG_DATA_DIR` value (or `None` to fall back to the platform
/// default). Always returns an absolute path so subprocess credential
/// helpers — which inherit our env but run in git's CWD, not ours —
/// don't end up looking for `tokens.json` in the user's repo. Factored
/// out so tests can exercise path resolution without poking at process
/// env state.
fn tokens_file_path_for(env_value: Option<&str>) -> PathBuf {
    let dir = env_value
        .map(PathBuf::from)
        .unwrap_or_else(default_data_dir);
    crate::git_credential::absolutize(&dir).join("tokens.json")
}

fn read_tokens() -> HashMap<String, String> {
    read_tokens_at(&tokens_file_path())
}

/// Read the token map, first tightening a pre-existing file to `0600`.
/// Stores written before the permission hardening landed sit at the umask
/// default (usually 0644) inside a bind-mounted `/data` volume; tightening on
/// every read is idempotent and cheap, and doing it BEFORE the read means no
/// code path ever handles token bytes from a world-readable file it could have
/// fixed. Best-effort: if chmod fails the read will usually fail too, and a
/// read-only mount is not made worse by proceeding.
fn read_tokens_at(path: &Path) -> HashMap<String, String> {
    read_tokens_at_checked(path).unwrap_or_default()
}

/// The same read, but an unreadable or corrupt store is an error rather than an
/// empty map. Callers that WRITE secrets back need the distinction: "there is no
/// entry" and "the file would not open" lead to opposite decisions on the way
/// out — write nothing, versus refuse to write at all — and collapsing them
/// turns a transient read failure into a permanent deletion.
fn read_tokens_at_checked(path: &Path) -> Result<HashMap<String, String>, String> {
    #[cfg(unix)]
    if path.exists() {
        use std::os::unix::fs::PermissionsExt;
        if let Err(err) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
            // Keep reading (a read-only mount is not made worse), but make the
            // failed hardening observable instead of silently world-readable.
            tracing::warn!(
                "[tokens] could not tighten {} to 0600: {err}",
                path.display()
            );
        }
    }
    match std::fs::read_to_string(path) {
        Ok(raw) => serde_json::from_str(&raw)
            .map_err(|e| format!("token store is not readable JSON: {e}")),
        // A store that was never written is legitimately empty.
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(HashMap::new()),
        Err(err) => Err(format!("token store read error: {err}")),
    }
}

fn write_tokens(tokens: &HashMap<String, String>) -> Result<(), String> {
    write_tokens_at(&tokens_file_path(), tokens)
}

/// Persist the token map without ever exposing a wide-permission file, even
/// transiently. A plain `fs::write` + chmod leaves a window (and a permanent
/// 0644 file if the process dies between the two), so on Unix the content goes
/// into a same-directory temp file created with mode `0600`, is fsynced, and
/// then atomically renamed over the store. The explicit `set_permissions`
/// after creation pins the bits exactly even under an exotic umask (umask can
/// only clear bits at open time; chmod is not masked).
fn write_tokens_at(path: &Path, tokens: &HashMap<String, String>) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "token store path has no parent directory".to_string())?;
    std::fs::create_dir_all(parent)
        .map_err(|e| format!("failed to create token store directory: {e}"))?;
    let json = serde_json::to_string_pretty(tokens)
        .map_err(|e| format!("failed to serialize tokens: {e}"))?;

    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        use std::sync::atomic::{AtomicU64, Ordering};
        static TMP_SEQ: AtomicU64 = AtomicU64::new(0);
        let tmp = parent.join(format!(
            ".tokens.json.tmp-{}-{}",
            std::process::id(),
            TMP_SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)
            .map_err(|e| format!("failed to create token store temp file: {e}"))?;
        let write_result = file
            .write_all(json.as_bytes())
            .and_then(|()| file.set_permissions(std::fs::Permissions::from_mode(0o600)))
            .and_then(|()| file.sync_all())
            .map_err(|e| format!("failed to write token store: {e}"))
            .and_then(|()| {
                std::fs::rename(&tmp, path)
                    .map_err(|e| format!("failed to persist token store: {e}"))
            });
        if write_result.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        write_result
    }

    #[cfg(not(unix))]
    {
        // Non-Unix server builds are outside the supported surface; keep the
        // plain write rather than pretending NTFS ACL hardening exists here.
        std::fs::write(path, json).map_err(|e| format!("failed to write token store: {e}"))
    }
}

#[cfg(feature = "tauri-runtime")]
fn keyring_get(key: &str) -> Option<String> {
    let entry = keyring::Entry::new(SERVICE_NAME, key).ok()?;
    entry.get_password().ok()
}

#[cfg(feature = "tauri-runtime")]
fn keyring_delete(key: &str) -> Result<(), String> {
    let entry =
        keyring::Entry::new(SERVICE_NAME, key).map_err(|e| format!("keyring init error: {e}"))?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(format!("keyring delete error: {e}")),
    }
}

fn get_stored(key: &str) -> Option<String> {
    if let Some(value) = read_tokens().get(key).cloned() {
        return Some(value);
    }
    #[cfg(feature = "tauri-runtime")]
    {
        // One-shot migration: copy a pre-file Keychain item into tokens.json
        // so later reads (and later app replacements) never touch Keychain.
        if let Some(value) = keyring_get(key) {
            let mut tokens = read_tokens();
            tokens.insert(key.to_string(), value.clone());
            if let Err(err) = write_tokens(&tokens) {
                tracing::warn!(
                    "[tokens] migrated keyring entry but failed to persist {key}: {err}"
                );
            } else {
                tracing::info!("[tokens] migrated keyring entry {key} to tokens.json");
            }
            return Some(value);
        }
    }
    None
}

fn set_stored(key: &str, value: &str) -> Result<(), String> {
    let mut tokens = read_tokens();
    tokens.insert(key.to_string(), value.to_string());
    write_tokens(&tokens)
}

fn delete_stored(key: &str) -> Result<(), String> {
    let mut tokens = read_tokens();
    tokens.remove(key);
    write_tokens(&tokens)?;
    #[cfg(feature = "tauri-runtime")]
    {
        // Best-effort: leftover Keychain items would otherwise be migrated
        // back onto the next get after a file delete.
        if let Err(err) = keyring_delete(key) {
            tracing::warn!("[tokens] failed to delete keyring leftover {key}: {err}");
        }
    }
    Ok(())
}

pub fn set_token(account_id: &str, token: &str) -> Result<(), String> {
    set_stored(&token_key(account_id), token)
}

pub fn get_token(account_id: &str) -> Option<String> {
    get_stored(&token_key(account_id))
}

pub fn delete_token(account_id: &str) -> Result<(), String> {
    delete_stored(&token_key(account_id))
}

pub fn set_channel_token(channel_id: i32, token: &str) -> Result<(), String> {
    set_stored(&channel_token_key(channel_id), token)
}

pub fn get_channel_token(channel_id: i32) -> Option<String> {
    get_stored(&channel_token_key(channel_id))
}

pub fn delete_channel_token(channel_id: i32) -> Result<(), String> {
    delete_stored(&channel_token_key(channel_id))
}

// ── Named secrets ──
// Same `0600` `tokens.json` as account and channel tokens, including on
// desktop. A feature secret (config-sync WebDAV password, snapshot passphrase)
// uses the `secret:` prefix so it cannot collide with those keys.
//
// `Ok(None)` is "nothing stored under that name"; `Err` is "the store would not
// open". Unlike [`get_token`], which collapses both into `None`, a secret's
// reader has to keep them apart: an empty value means "delete this entry" when
// it travels back through [`set_secret`]/[`delete_secret`], so a failed read
// reported as "absent" would erase the secret at the next save.

pub fn set_secret(name: &str, value: &str) -> Result<(), String> {
    set_stored(&secret_key(name), value)
}

pub fn get_secret(name: &str) -> Result<Option<String>, String> {
    Ok(read_tokens_at_checked(&tokens_file_path())?
        .get(&secret_key(name))
        .cloned())
}

pub fn delete_secret(name: &str) -> Result<(), String> {
    delete_stored(&secret_key(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tokens_file_path_absolutizes_relative_env() {
        // Regression: a relative `CODEG_DATA_DIR=data` previously made
        // `tokens.json` resolve against the helper subprocess's CWD (i.e.
        // git's repo dir), even after we'd absolutized the path used for
        // the database. The token store must always land on an absolute
        // path so DB lookup and token lookup point at the same root.
        let cwd = std::env::current_dir().expect("cwd");
        let resolved = tokens_file_path_for(Some("data"));
        assert!(
            resolved.is_absolute(),
            "tokens path must be absolute, got: {}",
            resolved.display()
        );
        assert_eq!(resolved, cwd.join("data").join("tokens.json"));
    }

    #[test]
    fn test_tokens_file_path_absolute_env_unchanged() {
        let data_dir = std::env::current_dir().expect("cwd").join("codeg-data");
        let data_dir_str = data_dir.to_string_lossy().to_string();
        let resolved = tokens_file_path_for(Some(&data_dir_str));
        assert_eq!(resolved, data_dir.join("tokens.json"));
    }

    #[test]
    fn test_tokens_file_path_default_when_unset() {
        // No env var → derived from `dirs::data_dir()` (always absolute on
        // every platform we ship to). Just verify we end at `tokens.json`
        // and that the result is absolute, not the literal default.
        let resolved = tokens_file_path_for(None);
        assert!(resolved.is_absolute());
        assert!(resolved.ends_with("tokens.json"));
    }

    #[cfg(unix)]
    fn mode_bits(path: &std::path::Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777
    }

    /// A fresh store must be 0600 from its very first byte on disk — there is
    /// no window where a parallel reader could see a wide-permission file,
    /// because the temp file is created with the final mode and only then
    /// renamed into place.
    #[test]
    #[cfg(unix)]
    fn test_write_tokens_creates_0600() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("tokens.json");
        let mut tokens = std::collections::HashMap::new();
        tokens.insert("github-token:a".to_string(), "secret".to_string());
        write_tokens_at(&path, &tokens).expect("write");
        assert_eq!(mode_bits(&path), 0o600);
        assert_eq!(
            read_tokens_at(&path).get("github-token:a").unwrap(),
            "secret"
        );
        // No temp residue left behind.
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with(".tokens.json.tmp")
            })
            .collect();
        assert!(leftovers.is_empty(), "temp files must not survive a write");
    }

    /// Overwriting a legacy wide-permission store must end 0600: the rename
    /// replaces the inode, so the old 0644 bits die with the old file.
    #[test]
    #[cfg(unix)]
    fn test_write_tokens_replaces_legacy_wide_file_with_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("tokens.json");
        std::fs::write(&path, "{}").expect("seed legacy file");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let mut tokens = std::collections::HashMap::new();
        tokens.insert("github-token:b".to_string(), "s2".to_string());
        write_tokens_at(&path, &tokens).expect("write");
        assert_eq!(mode_bits(&path), 0o600);
    }

    /// Reading an existing legacy store tightens it to 0600 before the bytes
    /// are consumed, so a server that only ever reads (never re-saves) still
    /// heals the volume-mounted file.
    #[test]
    #[cfg(unix)]
    fn test_read_tokens_tightens_existing_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("tokens.json");
        std::fs::write(&path, r#"{"github-token:c":"s3"}"#).expect("seed");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let tokens = read_tokens_at(&path);
        assert_eq!(tokens.get("github-token:c").unwrap(), "s3");
        assert_eq!(mode_bits(&path), 0o600);
    }

    #[test]
    fn file_store_round_trips_account_and_channel_tokens() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("tokens.json");
        let mut tokens = HashMap::new();
        tokens.insert(token_key("acct-1"), "ghp_secret".to_string());
        tokens.insert(channel_token_key(9), "lark-secret".to_string());
        write_tokens_at(&path, &tokens).expect("write");
        let loaded = read_tokens_at(&path);
        assert_eq!(loaded.get(&token_key("acct-1")).unwrap(), "ghp_secret");
        assert_eq!(loaded.get(&channel_token_key(9)).unwrap(), "lark-secret");
    }
}
