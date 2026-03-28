//! Identity management for worker nodes.
//!
//! Provides `--generate-identity` and `--print-peer-id` CLI commands.

use anyhow::Result;
use willow_identity::Identity;

/// Generate a new Ed25519 identity and save it to `path`.
///
/// If a file already exists at `path`, returns an error to prevent
/// accidental overwrites.
pub fn generate_identity(path: &str) -> Result<Identity> {
    let p = std::path::Path::new(path);
    if p.exists() {
        anyhow::bail!("identity file already exists: {path}");
    }
    let identity = Identity::load_or_generate(path)?;
    Ok(identity)
}

/// Load an existing identity from `path`, or generate if absent.
pub fn load_or_generate(path: &str) -> Result<Identity> {
    Identity::load_or_generate(path).map_err(|e| anyhow::anyhow!("failed to load identity: {e}"))
}

/// Print the peer ID for an identity file. Used by operators to
/// collect worker peer IDs for `PLATFORM_WORKERS`.
pub fn print_peer_id(path: &str) -> Result<()> {
    let identity = load_or_generate(path)?;
    println!("{}", identity.peer_id());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn generate_creates_new_identity() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.key");
        let path_str = path.to_str().unwrap();

        let id = generate_identity(path_str).unwrap();
        assert!(path.exists());
        assert!(!id.peer_id().to_string().is_empty());
    }

    #[test]
    fn generate_refuses_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("existing.key");
        fs::write(&path, b"existing data").unwrap();

        let result = generate_identity(path.to_str().unwrap());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("already exists"));
    }

    #[test]
    fn load_or_generate_creates_if_absent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("new.key");
        let path_str = path.to_str().unwrap();

        let id = load_or_generate(path_str).unwrap();
        assert!(path.exists());
        assert!(!id.peer_id().to_string().is_empty());
    }

    #[test]
    fn load_or_generate_reloads_same_identity() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("reload.key");
        let path_str = path.to_str().unwrap();

        let id1 = load_or_generate(path_str).unwrap();
        let id2 = load_or_generate(path_str).unwrap();
        assert_eq!(id1.peer_id().to_string(), id2.peer_id().to_string());
    }
}
