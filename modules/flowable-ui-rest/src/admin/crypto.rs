//! AES/CBC/PKCS5 (PKCS7) password encryption matching Java
//! `AbstractEncryptingService` defaults from flowable-default.properties.

use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};

type Aes128CbcEnc = cbc::Encryptor<aes::Aes128>;
type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;

/// Default IV / secret from Java `flowable-default.properties` (16 ASCII chars each).
pub const DEFAULT_IV: &str = "j8kdO2hejA9lKmm6";
pub const DEFAULT_SECRET: &str = "9FGl73ngxcOoJvmL";

#[derive(Clone, Debug)]
pub struct PasswordCipher {
    iv: [u8; 16],
    key: [u8; 16],
}

impl Default for PasswordCipher {
    fn default() -> Self {
        Self::from_specs(DEFAULT_IV, DEFAULT_SECRET).expect("default AES specs are 16 bytes")
    }
}

impl PasswordCipher {
    pub fn from_env() -> Self {
        let iv = std::env::var("FLOWABLE_ADMIN_CREDENTIALS_IV")
            .unwrap_or_else(|_| DEFAULT_IV.to_string());
        let secret = std::env::var("FLOWABLE_ADMIN_CREDENTIALS_SECRET")
            .unwrap_or_else(|_| DEFAULT_SECRET.to_string());
        Self::from_specs(&iv, &secret).unwrap_or_else(|_| Self::default())
    }

    pub fn from_specs(iv: &str, secret: &str) -> Result<Self, String> {
        let iv_bytes = iv.as_bytes();
        let key_bytes = secret.as_bytes();
        if iv_bytes.len() != 16 {
            return Err(format!(
                "credentials IV must be 16 bytes, got {}",
                iv_bytes.len()
            ));
        }
        if key_bytes.len() != 16 {
            return Err(format!(
                "credentials secret must be 16 bytes, got {}",
                key_bytes.len()
            ));
        }
        let mut iv_arr = [0u8; 16];
        let mut key_arr = [0u8; 16];
        iv_arr.copy_from_slice(iv_bytes);
        key_arr.copy_from_slice(key_bytes);
        Ok(Self {
            iv: iv_arr,
            key: key_arr,
        })
    }

    pub fn encrypt(&self, value: &str) -> Result<String, String> {
        let encryptor = Aes128CbcEnc::new((&self.key).into(), (&self.iv).into());
        let ciphertext = encryptor
            .encrypt_padded_vec_mut::<Pkcs7>(value.as_bytes());
        Ok(B64.encode(ciphertext))
    }

    pub fn decrypt(&self, encrypted: &str) -> Result<String, String> {
        let raw = B64
            .decode(encrypted.as_bytes())
            .map_err(|e| format!("base64 decode failed: {e}"))?;
        let decryptor = Aes128CbcDec::new((&self.key).into(), (&self.iv).into());
        let plain = decryptor
            .decrypt_padded_vec_mut::<Pkcs7>(&raw)
            .map_err(|e| format!("AES decrypt failed: {e}"))?;
        String::from_utf8(plain).map_err(|e| format!("utf-8 decode failed: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let cipher = PasswordCipher::default();
        let enc = cipher.encrypt("test").unwrap();
        assert_ne!(enc, "test");
        assert_eq!(cipher.decrypt(&enc).unwrap(), "test");
    }
}
