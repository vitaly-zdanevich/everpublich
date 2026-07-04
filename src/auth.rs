//! Admin-session tokens derived from the Evernote OAuth token.
//!
//! The browser should not store the raw Evernote token. Instead, after OAuth the
//! backend stores the encrypted Evernote token in DynamoDB and returns an
//! HMAC-signed session token. The admin panel sends that session token back.

use anyhow::{Context, Result, bail};
use base64::Engine;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

const VERSION: &str = "adm1";

/// Produce a short stable fingerprint used in signed admin-session tokens.
pub fn evernote_token_fingerprint(evernote_token: &str) -> String {
	let digest = Sha256::digest(evernote_token.as_bytes());
	base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&digest[..16])
}

/// Build a browser-storable admin token for one user.
pub fn session_token(user_id: &str, evernote_token: &str, secret: &str) -> Result<String> {
	let fingerprint = evernote_token_fingerprint(evernote_token);
	let payload = format!("{VERSION}.{user_id}.{fingerprint}");
	let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).context("invalid HMAC key")?;
	mac.update(payload.as_bytes());
	let sig = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
	Ok(format!("{payload}.{sig}"))
}

/// Verify a session token against the current encrypted Evernote token.
pub fn verify_session(
	token: &str,
	expected_user_id: &str,
	current_evernote_token: &str,
	secret: &str,
) -> Result<()> {
	let parts = token.split('.').collect::<Vec<_>>();
	if parts.len() != 4 || parts[0] != VERSION {
		bail!("invalid admin session token");
	}
	if parts[1] != expected_user_id {
		bail!("admin token belongs to another user");
	}
	if parts[2] != evernote_token_fingerprint(current_evernote_token) {
		bail!("admin token does not match current Evernote token");
	}

	let expected = session_token(expected_user_id, current_evernote_token, secret)?;
	if subtle_eq(token.as_bytes(), expected.as_bytes()) {
		Ok(())
	} else {
		bail!("invalid admin token signature");
	}
}

fn subtle_eq(a: &[u8], b: &[u8]) -> bool {
	if a.len() != b.len() {
		return false;
	}
	let mut out = 0u8;
	for (x, y) in a.iter().zip(b) {
		out |= x ^ y;
	}
	out == 0
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn verifies_current_token() {
		let token = session_token("u1", "evernote-token", "server-secret").unwrap();
		verify_session(&token, "u1", "evernote-token", "server-secret").unwrap();
		assert!(verify_session(&token, "u2", "evernote-token", "server-secret").is_err());
		assert!(verify_session(&token, "u1", "rotated-token", "server-secret").is_err());
	}
}
