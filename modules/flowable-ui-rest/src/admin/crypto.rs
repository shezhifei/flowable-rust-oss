// Pre-existing `unwrap()` call(s), grandfathered by the workspace clippy ratchet
// (`[workspace.lints.clippy] unwrap_used = "warn"` in the root Cargo.toml). These
// sites predate the ratchet and were NOT individually audited against Java. The
// exemption is scoped with `cfg_attr(test, ...)`, so it covers only this file's
// `#[cfg(test)]` code; a NEW unwrap() in production code is still surfaced.
// Do not add more without an audit note.
#![cfg_attr(test, allow(clippy::unwrap_used))]

//! AES/CBC/PKCS5 (PKCS7) password encryption matching Java
//! `AbstractEncryptingService` defaults from flowable-default.properties.

use aes::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit, block_padding::Pkcs7};
use base64::{Engine as _, engine::general_purpose::STANDARD as B64};

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
        // E4 SC3 exemption: DEFAULT_IV / DEFAULT_SECRET are compile-time
        // 16-byte constants; from_specs cannot fail for them. Public entry
        // `from_specs`/`from_env` return Result (fail-closed).
        #[allow(
            clippy::expect_used,
            reason = "E4: Default uses compile-time 16-byte constants; from_specs is infallible here"
        )]
        let cipher =
            Self::from_specs(DEFAULT_IV, DEFAULT_SECRET).expect("default AES specs are 16 bytes");
        cipher
    }
}

impl PasswordCipher {
    /// Builds the cipher from `FLOWABLE_ADMIN_CREDENTIALS_IV` /
    /// `FLOWABLE_ADMIN_CREDENTIALS_SECRET`, falling back to the built-in
    /// development defaults only when the variables are *unset*.
    ///
    /// Fail-closed by design (P1-D): a variable that is set but not exactly 16
    /// bytes is an operator misconfiguration and returns an `Err` naming the
    /// offending variable instead of silently using the public default key.
    /// Java `AbstractEncryptingService` has the same boundary — the
    /// `IvParameterSpec` constructor throws `IllegalArgumentException` for a
    /// non-16-byte IV and a bad key length surfaces from `cipher.init`, both
    /// aborting Spring bean construction (`.../admin/service/engine/
    /// AbstractEncryptingService.java:39-46,54-56,66-68`); there is no
    /// "invalid config -> default key" branch on either side.
    pub fn from_env() -> Result<Self, String> {
        let iv = std::env::var("FLOWABLE_ADMIN_CREDENTIALS_IV")
            .unwrap_or_else(|_| DEFAULT_IV.to_string());
        let secret = std::env::var("FLOWABLE_ADMIN_CREDENTIALS_SECRET")
            .unwrap_or_else(|_| DEFAULT_SECRET.to_string());
        Self::from_specs(&iv, &secret).map_err(|error| {
            format!(
                "invalid FLOWABLE_ADMIN_CREDENTIALS_IV / FLOWABLE_ADMIN_CREDENTIALS_SECRET \
                 configuration: {error}; both values must be exactly 16 bytes (AES-128/CBC). \
                 Either fix the lengths or unset both variables to use the built-in development \
                 defaults — refusing to fall back to the default key, because server credentials \
                 would then be encrypted with a publicly known key"
            )
        })
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
        let ciphertext = encryptor.encrypt_padded_vec_mut::<Pkcs7>(value.as_bytes());
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

    #[test]
    fn from_specs_rejects_non_16_byte_lengths() {
        let err = PasswordCipher::from_specs("short", DEFAULT_SECRET).unwrap_err();
        assert!(err.contains("IV must be 16 bytes"), "{err}");

        let err = PasswordCipher::from_specs(DEFAULT_IV, "0123456789abcdef0").unwrap_err();
        assert!(err.contains("secret must be 16 bytes"), "{err}");
    }

    #[test]
    fn from_env_ok_on_unset_and_fail_closed_on_bad_lengths() {
        // This is the only test in this binary that touches these process-global
        // variables, so mutate and restore them inside one test (shared-library
        // unit tests run on parallel threads). SAFETY: single-writer test; both
        // variables are removed before every expectation and on exit.
        unsafe {
            std::env::remove_var("FLOWABLE_ADMIN_CREDENTIALS_IV");
            std::env::remove_var("FLOWABLE_ADMIN_CREDENTIALS_SECRET");
        }

        // Unset -> built-in defaults are a legitimate, working configuration.
        let cipher = PasswordCipher::from_env().expect("unset env yields the default cipher");
        let enc = cipher.encrypt("test").unwrap();
        assert_eq!(cipher.decrypt(&enc).unwrap(), "test");

        // Set but wrong length -> Err naming the variable, never a silent default.
        unsafe {
            std::env::set_var("FLOWABLE_ADMIN_CREDENTIALS_IV", "short");
        }
        let iv_err = PasswordCipher::from_env().expect_err("bad IV length must fail closed");
        unsafe {
            std::env::remove_var("FLOWABLE_ADMIN_CREDENTIALS_IV");
        }
        assert!(
            iv_err.contains("FLOWABLE_ADMIN_CREDENTIALS_IV") && iv_err.contains("16 bytes"),
            "{iv_err}"
        );

        unsafe {
            std::env::set_var("FLOWABLE_ADMIN_CREDENTIALS_SECRET", "0123456789abcdef0");
        }
        let secret_err =
            PasswordCipher::from_env().expect_err("bad secret length must fail closed");
        unsafe {
            std::env::remove_var("FLOWABLE_ADMIN_CREDENTIALS_SECRET");
        }
        assert!(
            secret_err.contains("FLOWABLE_ADMIN_CREDENTIALS_SECRET")
                && secret_err.contains("16 bytes"),
            "{secret_err}"
        );

        // Restored unset state is usable again.
        PasswordCipher::from_env().expect("unset env yields the default cipher after restore");
    }
}
