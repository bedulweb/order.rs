//! BigSeller frontend password blob (CryptoJS AES-CBC + PKCS7).
//!
//! Format:
//! ```text
//! "0" + hex(random 2 bytes) + hex(iv 16) + hex(key 16) + hex(ciphertext)
//! ```

use crate::error::{Error, Result};
use aes::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyIvInit};
use aes::Aes128;
use cbc::Encryptor;
use rand::RngCore;

type Aes128CbcEnc = Encryptor<Aes128>;

/// Encrypt a plaintext password the way BigSeller's Nuxt bundle does.
pub fn encrypt_password(plain: &str) -> Result<String> {
    let mut key = [0u8; 16];
    let mut iv = [0u8; 16];
    let mut rnd2 = [0u8; 2];
    let mut rng = rand::thread_rng();
    rng.fill_bytes(&mut key);
    rng.fill_bytes(&mut iv);
    rng.fill_bytes(&mut rnd2);

    let cipher = Aes128CbcEnc::new(&key.into(), &iv.into());
    let mut buf = plain.as_bytes().to_vec();
    // PKCS7 needs room for a full block of padding in the worst case.
    let pad_len = 16 - (buf.len() % 16);
    buf.extend(std::iter::repeat(0u8).take(pad_len));
    let ciphertext = cipher
        .encrypt_padded_mut::<Pkcs7>(&mut buf, plain.len())
        .map_err(|e| Error::Crypto(format!("AES encrypt failed: {e:?}")))?;

    Ok(format!(
        "0{}{}{}{}",
        hex::encode(rnd2),
        hex::encode(iv),
        hex::encode(key),
        hex::encode(ciphertext)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blob_shape() {
        let blob = encrypt_password("@Mastah123").expect("encrypt");
        assert!(blob.starts_with('0'));
        // 1 + 4 + 32 + 32 + ciphertext hex
        assert!(blob.len() > 1 + 4 + 32 + 32);
        assert!(blob[1..].chars().all(|c| c.is_ascii_hexdigit()));
    }
}
