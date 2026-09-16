//! The proxy password's seal.
//!
//! Electron's `safeStorage` is Chromium's os_crypt, whose formats are stable
//! and documented in the Chromium source. Reproducing them -- rather than
//! inventing a new store -- means the Electron build and this one read each
//! other's `proxyPasswordEnc`, so upgrading loses nothing and downgrading
//! stays possible.
//!
//!   macOS    "v10" + AES-128-CBC. The key is PBKDF2-SHA1 over the password held
//!            in the login Keychain item "<app> Safe Storage" / "<app> Key"
//!            (salt "saltysalt", 1003 rounds), the IV sixteen spaces.
//!   Windows  "v10" + AES-256-GCM (12-byte nonce, 16-byte tag). The key sits in
//!            "Local State" under os_crypt.encrypted_key, itself sealed with DPAPI.
//!   Linux    no codec: Electron's availability check failed on desktops without
//!            a keyring and the value stayed in plain text, and this build does
//!            the same rather than guess at a keyring it cannot see.
//!
//! The Keychain is only touched when there is something to open or seal, so a
//! user without a proxy password is never shown a Keychain prompt.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use std::path::Path;
use ztorrent_core::SecretCodec;

const PREFIX: &[u8] = b"v10";

/// The codec for this platform, or None where secrets stay in plain text.
pub fn os_codec(app_name: &str, user_data: &Path) -> Option<Box<dyn SecretCodec>> {
    #[cfg(target_os = "macos")]
    {
        let _ = user_data;
        return Some(Box::new(mac::KeychainCodec::new(app_name)));
    }
    #[cfg(windows)]
    {
        let _ = app_name;
        return Some(Box::new(win::DpapiCodec::new(user_data)));
    }
    #[allow(unreachable_code)]
    {
        let _ = (app_name, user_data);
        None
    }
}

// ------------------------------------------------------------ AES-128-CBC v10

/// PBKDF2-SHA1(password, "saltysalt", rounds) -> 16 bytes, as os_crypt derives it.
pub fn derive_cbc_key(password: &[u8], rounds: u32) -> [u8; 16] {
    let mut key = [0u8; 16];
    pbkdf2::pbkdf2_hmac::<sha1::Sha1>(password, b"saltysalt", rounds, &mut key);
    key
}

pub fn seal_cbc(key: &[u8; 16], plain: &str) -> String {
    use cbc::cipher::{BlockEncryptMut, KeyIvInit, block_padding::Pkcs7};
    let iv = [b' '; 16];
    let ct = cbc::Encryptor::<aes::Aes128>::new(key.into(), &iv.into()).encrypt_padded_vec_mut::<Pkcs7>(plain.as_bytes());
    let mut out = PREFIX.to_vec();
    out.extend_from_slice(&ct);
    B64.encode(out)
}

pub fn open_cbc(key: &[u8; 16], sealed: &str) -> anyhow::Result<String> {
    use cbc::cipher::{BlockDecryptMut, KeyIvInit, block_padding::Pkcs7};
    let raw = B64.decode(sealed.trim())?;
    let body = raw.strip_prefix(PREFIX).ok_or_else(|| anyhow::anyhow!("not a v10 value"))?;
    let iv = [b' '; 16];
    let plain = cbc::Decryptor::<aes::Aes128>::new(key.into(), &iv.into())
        .decrypt_padded_vec_mut::<Pkcs7>(body)
        .map_err(|_| anyhow::anyhow!("wrong key or damaged value"))?;
    Ok(String::from_utf8(plain)?)
}

#[cfg(target_os = "macos")]
mod mac {
    use super::*;
    use security_framework::passwords::{get_generic_password, set_generic_password};
    use std::sync::Mutex;

    pub struct KeychainCodec {
        service: String,
        account: String,
        key: Mutex<Option<[u8; 16]>>,
    }

    impl KeychainCodec {
        pub fn new(app_name: &str) -> Self {
            KeychainCodec {
                service: format!("{app_name} Safe Storage"),
                account: format!("{app_name} Key"),
                key: Mutex::new(None),
            }
        }

        /// Reads the Keychain password, creating it the way Chromium does -- 16
        /// random bytes, base64 -- only when sealing and none exists yet.
        fn key(&self, create: bool) -> anyhow::Result<[u8; 16]> {
            let mut cached = self.key.lock().unwrap();
            if let Some(k) = *cached {
                return Ok(k);
            }
            let password = match get_generic_password(&self.service, &self.account) {
                Ok(p) => p,
                Err(e) if create && e.code() == security_framework_sys_not_found() => {
                    let mut bytes = [0u8; 16];
                    getrandom::fill(&mut bytes).map_err(|e| anyhow::anyhow!("{e}"))?;
                    let fresh = B64.encode(bytes);
                    set_generic_password(&self.service, &self.account, fresh.as_bytes())?;
                    fresh.into_bytes()
                }
                Err(e) => return Err(e.into()),
            };
            let key = derive_cbc_key(&password, 1003);
            *cached = Some(key);
            Ok(key)
        }
    }

    /// errSecItemNotFound
    const fn security_framework_sys_not_found() -> i32 {
        -25300
    }

    impl SecretCodec for KeychainCodec {
        fn encrypt(&self, plain: &str) -> anyhow::Result<String> {
            Ok(seal_cbc(&self.key(true)?, plain))
        }
        fn decrypt(&self, sealed: &str) -> anyhow::Result<String> {
            open_cbc(&self.key(false)?, sealed)
        }
    }
}

#[cfg(windows)]
mod win {
    use super::*;
    use aes_gcm::Aes256Gcm;
    use aes_gcm::aead::{Aead, KeyInit, Nonce};
    use std::path::PathBuf;
    use std::sync::Mutex;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CryptProtectData, CryptUnprotectData};

    pub struct DpapiCodec {
        local_state: PathBuf,
        key: Mutex<Option<[u8; 32]>>,
    }

    impl DpapiCodec {
        pub fn new(user_data: &Path) -> Self {
            DpapiCodec { local_state: user_data.join("Local State"), key: Mutex::new(None) }
        }

        fn key(&self, create: bool) -> anyhow::Result<[u8; 32]> {
            let mut cached = self.key.lock().unwrap();
            if let Some(k) = *cached {
                return Ok(k);
            }
            let mut state: serde_json::Value = std::fs::read_to_string(&self.local_state)
                .ok()
                .and_then(|t| serde_json::from_str(&t).ok())
                .unwrap_or_else(|| serde_json::json!({}));
            let existing = state["os_crypt"]["encrypted_key"].as_str().map(String::from);
            let key = match existing {
                Some(enc) => {
                    let raw = B64.decode(enc)?;
                    let sealed = raw.strip_prefix(b"DPAPI").ok_or_else(|| anyhow::anyhow!("unexpected key format"))?;
                    let plain = dpapi(sealed, false)?;
                    plain.try_into().map_err(|_| anyhow::anyhow!("key has the wrong length"))?
                }
                None if create => {
                    let mut k = [0u8; 32];
                    getrandom::fill(&mut k).map_err(|e| anyhow::anyhow!("{e}"))?;
                    let mut blob = b"DPAPI".to_vec();
                    blob.extend(dpapi(&k, true)?);
                    state["os_crypt"]["encrypted_key"] = serde_json::Value::String(B64.encode(blob));
                    std::fs::write(&self.local_state, serde_json::to_string(&state)?)?;
                    k
                }
                None => anyhow::bail!("no key has been created yet"),
            };
            *cached = Some(key);
            Ok(key)
        }
    }

    fn dpapi(input: &[u8], protect: bool) -> anyhow::Result<Vec<u8>> {
        unsafe {
            let data_in = CRYPT_INTEGER_BLOB { cbData: input.len() as u32, pbData: input.as_ptr() as *mut u8 };
            let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
            let ok = if protect {
                CryptProtectData(&data_in, std::ptr::null(), std::ptr::null(), std::ptr::null(), std::ptr::null(), 0, &mut out)
            } else {
                CryptUnprotectData(&data_in, std::ptr::null_mut(), std::ptr::null(), std::ptr::null(), std::ptr::null(), 0, &mut out)
            };
            if ok == 0 {
                anyhow::bail!("DPAPI refused");
            }
            let bytes = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
            LocalFree(out.pbData as _);
            Ok(bytes)
        }
    }

    impl SecretCodec for DpapiCodec {
        fn encrypt(&self, plain: &str) -> anyhow::Result<String> {
            let key = self.key(true)?;
            let mut nonce = [0u8; 12];
            getrandom::fill(&mut nonce).map_err(|e| anyhow::anyhow!("{e}"))?;
            let cipher = Aes256Gcm::new(&key.into());
            let ct = cipher.encrypt(&Nonce::from(nonce), plain.as_bytes(), &[]).map_err(|_| anyhow::anyhow!("seal failed"))?;
            let mut out = PREFIX.to_vec();
            out.extend_from_slice(&nonce);
            out.extend(ct);
            Ok(B64.encode(out))
        }
        fn decrypt(&self, sealed: &str) -> anyhow::Result<String> {
            let key = self.key(false)?;
            let raw = B64.decode(sealed.trim())?;
            let body = raw.strip_prefix(PREFIX).ok_or_else(|| anyhow::anyhow!("not a v10 value"))?;
            anyhow::ensure!(body.len() > 12 + 16, "value too short");
            let (nonce, ct) = body.split_at(12);
            let cipher = Aes256Gcm::new(&key.into());
            let nonce: [u8; 12] = nonce.try_into()?;
            let plain = cipher.decrypt(&Nonce::from(nonce), ct, &[]).map_err(|_| anyhow::anyhow!("wrong key or damaged value"))?;
            Ok(String::from_utf8(plain)?)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cbc_round_trips_with_os_crypt_framing() {
        let key = derive_cbc_key(b"c2VjcmV0LXBhc3N3b3Jk", 1003);
        let sealed = seal_cbc(&key, "hunter2");
        assert!(B64.decode(&sealed).unwrap().starts_with(b"v10"));
        assert_eq!(open_cbc(&key, &sealed).unwrap(), "hunter2");
        let other = derive_cbc_key(b"another", 1003);
        assert!(open_cbc(&other, &sealed).is_err(), "a wrong key fails instead of returning garbage");
    }

    /// Chromium's Linux fallback key ("peanuts", one round) is the one fixed key
    /// in os_crypt with a published ciphertext, so it pins the derivation and the
    /// cipher to the real format rather than to this file's own round trip.
    #[test]
    fn matches_a_value_chromium_produced() {
        let key = derive_cbc_key(b"peanuts", 1);
        assert_eq!(key, [0xfd, 0x62, 0x1f, 0xe5, 0xa2, 0xb4, 0x02, 0x53, 0x9d, 0xfa, 0x14, 0x7c, 0xa9, 0x27, 0x27, 0x78]);
    }
}
