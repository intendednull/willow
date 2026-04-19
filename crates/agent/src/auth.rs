//! # Bearer Token Generation
//!
//! Generate and manage bearer tokens for MCP transport authentication.
//! Stdio transport skips auth (process isolation). SSE/HTTP transports
//! require a bearer token in the `Authorization` header.

use rand::Rng;

/// Token prefix for Willow agent tokens.
const TOKEN_PREFIX: &str = "wlw_";

/// Generate a 256-bit random bearer token with `wlw_` prefix.
pub fn generate_token() -> String {
    let mut rng = rand::thread_rng();
    let mut bytes = [0u8; 32];
    rng.fill(&mut bytes);
    format!("{}{}", TOKEN_PREFIX, hex::encode(&bytes))
}

/// Resolve the bearer token: use provided value, or generate one.
/// If `token_file` is set, write the token to that path with 0600 permissions.
pub fn resolve_token(token: &Option<String>, token_file: Option<&str>) -> anyhow::Result<String> {
    let t = match token {
        Some(t) => t.clone(),
        None => generate_token(),
    };

    if let Some(path) = token_file {
        std::fs::write(path, &t)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }
        tracing::info!("token written to {}", path);
    }

    Ok(t)
}

/// Simple hex encoding (avoids pulling in the `hex` crate for just this).
mod hex {
    pub fn encode(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_has_correct_prefix() {
        let token = generate_token();
        assert!(token.starts_with("wlw_"));
    }

    #[test]
    fn token_has_correct_length() {
        let token = generate_token();
        // wlw_ (4) + 64 hex chars = 68
        assert_eq!(token.len(), 68);
    }

    #[test]
    fn tokens_are_unique() {
        let t1 = generate_token();
        let t2 = generate_token();
        assert_ne!(t1, t2);
    }

    #[test]
    fn resolve_uses_provided_token() {
        let provided = "wlw_custom".to_string();
        let result = resolve_token(&Some(provided.clone()), None).unwrap();
        assert_eq!(result, provided);
    }

    #[test]
    fn resolve_generates_when_none() {
        let result = resolve_token(&None, None).unwrap();
        assert!(result.starts_with("wlw_"));
        assert_eq!(result.len(), 68);
    }

    #[test]
    fn token_file_written_and_readable() {
        let dir = std::env::temp_dir().join("willow-auth-test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("test-token");
        let path_str = path.to_string_lossy().into_owned();
        let token = resolve_token(&None, Some(&path_str)).unwrap();
        let read_back = std::fs::read_to_string(&path).unwrap();
        assert_eq!(token, read_back);
        let _ = std::fs::remove_file(&path);
    }
}
