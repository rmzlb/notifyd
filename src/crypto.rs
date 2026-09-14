use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::STANDARD, Engine};

pub fn load_key() -> Result<[u8; 32]> {
    let raw = std::env::var("TELEGRAM_ENCRYPTION_KEY")
        .map_err(|_| anyhow!("TELEGRAM_ENCRYPTION_KEY not set"))?;
    let bytes = STANDARD
        .decode(raw.trim())
        .map_err(|e| anyhow!("TELEGRAM_ENCRYPTION_KEY base64 decode: {e}"))?;
    bytes
        .try_into()
        .map_err(|_| anyhow!("TELEGRAM_ENCRYPTION_KEY must be 32 bytes"))
}

pub fn encrypt(key: &[u8; 32], plaintext: &[u8]) -> Result<Vec<u8>> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| anyhow!("{e}"))?;
    let nonce_bytes: [u8; 12] = uuid::Uuid::new_v4().as_bytes()[..12].try_into().unwrap();
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| anyhow!("encrypt: {e}"))?;
    let mut out = Vec::with_capacity(12 + ciphertext.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

pub fn decrypt(key: &[u8; 32], blob: &[u8]) -> Result<Vec<u8>> {
    if blob.len() < 12 {
        return Err(anyhow!("credential unavailable"));
    }
    let (nonce_bytes, ciphertext) = blob.split_at(12);
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| anyhow!("{e}"))?;
    let nonce = Nonce::from_slice(nonce_bytes);
    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| anyhow!("credential unavailable"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> [u8; 32] {
        [0x42u8; 32]
    }

    #[test]
    fn round_trip() {
        let key = test_key();
        let plain = b"bot:1234567890:AAAAA";
        let blob = encrypt(&key, plain).unwrap();
        let decrypted = decrypt(&key, &blob).unwrap();
        assert_eq!(decrypted, plain);
    }

    #[test]
    fn ciphertext_corruption() {
        let key = test_key();
        let mut blob = encrypt(&key, b"hello").unwrap();
        let last = blob.len() - 1;
        blob[last] ^= 0xff;
        assert!(decrypt(&key, &blob).is_err());
    }

    #[test]
    fn wrong_key() {
        let key1 = test_key();
        let key2 = [0x99u8; 32];
        let blob = encrypt(&key1, b"hello").unwrap();
        assert!(decrypt(&key2, &blob).is_err());
    }
}
