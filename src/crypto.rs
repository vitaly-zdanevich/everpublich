//! Low-cost token encryption for the MVP.
//!
//! This intentionally avoids paid key-management services because the project starts as a free pilot.
//! It uses AES-256-GCM from `ring`; the key is derived from the
//! `EVERPUBLICH_TOKEN_SECRET` environment variable. Operationally this means the
//! secret must be rotated like any other production secret, and leaked runtime
//! environment variables can decrypt stored tokens.

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use ring::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
use ring::rand::{SecureRandom, SystemRandom};
use sha2::{Digest, Sha256};

const VERSION: &str = "v1";
const NONCE_LEN: usize = 12;

/// Encrypts/decrypts OAuth tokens with an environment-supplied secret.
pub struct TokenCipher {
	key: LessSafeKey,
}

impl TokenCipher {
	/// Create a cipher from an operator-provided secret.
	pub fn from_secret(secret: &str) -> Result<Self> {
		if secret.trim().len() < 32 {
			bail!("EVERPUBLICH_TOKEN_SECRET must be at least 32 characters");
		}

		let digest = Sha256::digest(secret.as_bytes());
		let unbound = UnboundKey::new(&AES_256_GCM, &digest)
			.map_err(|_| anyhow!("failed to initialize AES-256-GCM key"))?;
		Ok(Self {
			key: LessSafeKey::new(unbound),
		})
	}

	/// Encrypt one token string for storage in SQLite.
	pub fn encrypt(&self, plaintext: &str) -> Result<String> {
		let mut nonce = [0u8; NONCE_LEN];
		SystemRandom::new()
			.fill(&mut nonce)
			.map_err(|_| anyhow!("failed to generate encryption nonce"))?;

		let mut bytes = plaintext.as_bytes().to_vec();
		self.key
			.seal_in_place_append_tag(
				Nonce::assume_unique_for_key(nonce),
				Aad::empty(),
				&mut bytes,
			)
			.map_err(|_| anyhow!("failed to encrypt token"))?;

		let mut packed = Vec::with_capacity(NONCE_LEN + bytes.len());
		packed.extend_from_slice(&nonce);
		packed.extend_from_slice(&bytes);
		Ok(format!(
			"{VERSION}.{}",
			base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(packed)
		))
	}

	/// Decrypt a token previously produced by [`TokenCipher::encrypt`].
	pub fn decrypt(&self, encoded: &str) -> Result<String> {
		let Some(data) = encoded.strip_prefix(&format!("{VERSION}.")) else {
			bail!("unsupported token ciphertext version");
		};
		let mut packed = base64::engine::general_purpose::URL_SAFE_NO_PAD
			.decode(data)
			.context("token ciphertext is not valid base64")?;
		if packed.len() <= NONCE_LEN {
			bail!("token ciphertext is too short");
		}

		let mut nonce = [0u8; NONCE_LEN];
		nonce.copy_from_slice(&packed[..NONCE_LEN]);
		let mut body = packed.split_off(NONCE_LEN);
		let plain = self
			.key
			.open_in_place(Nonce::assume_unique_for_key(nonce), Aad::empty(), &mut body)
			.map_err(|_| anyhow!("failed to decrypt token"))?;
		String::from_utf8(plain.to_vec()).context("decrypted token is not UTF-8")
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn encrypts_and_decrypts() {
		let cipher = TokenCipher::from_secret("0123456789abcdef0123456789abcdef").unwrap();
		let encrypted = cipher.encrypt("evernote-token").unwrap();

		assert_ne!(encrypted, "evernote-token");
		assert_eq!(cipher.decrypt(&encrypted).unwrap(), "evernote-token");
	}

	#[test]
	fn rejects_short_secret() {
		assert!(TokenCipher::from_secret("short").is_err());
	}
}
