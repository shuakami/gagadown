use ring::aead::{self, Aad, LessSafeKey, Nonce, UnboundKey};
use ring::rand::{SecureRandom, SystemRandom};

#[cfg(not(feature = "ci-test-keyring"))]
const SERVICE: &str = "dev.gagadown.GaGaDown";
#[cfg(not(feature = "ci-test-keyring"))]
const USER: &str = "task-state-encryption-key";
#[cfg(not(feature = "ci-test-keyring"))]
const API_USER: &str = "extension-api-token";
const PREFIX: &[u8] = b"GAGADOWN-STATE-ENC-1\0";
const AAD: &[u8] = b"gagadown-task-state-v1";

#[cfg(all(feature = "ci-test-keyring", not(debug_assertions)))]
compile_error!("ci-test-keyring must never be enabled in release builds");

pub struct StateCipher {
    key: LessSafeKey,
}

impl StateCipher {
    pub fn load_or_create() -> Result<Self, String> {
        #[cfg(feature = "ci-test-keyring")]
        return Self::from_key_material(&[0x42; 32]).map_err(|e| e.to_string());

        #[cfg(not(feature = "ci-test-keyring"))]
        {
            let entry = keyring::Entry::new(SERVICE, USER).map_err(|e| e.to_string())?;
            let material = match entry.get_password() {
                Ok(value) => {
                    hex::decode(value).map_err(|e| format!("invalid keychain key: {e}"))?
                }
                Err(keyring::Error::NoEntry) => {
                    let mut key = [0; 32];
                    SystemRandom::new()
                        .fill(&mut key)
                        .map_err(|_| "secure random generator unavailable")?;
                    entry
                        .set_password(&hex::encode(key))
                        .map_err(|e| e.to_string())?;
                    key.to_vec()
                }
                Err(e) => return Err(e.to_string()),
            };
            Self::from_key_material(&material)
        }
    }

    fn from_key_material(material: &[u8]) -> Result<Self, String> {
        let key =
            UnboundKey::new(&aead::AES_256_GCM, material).map_err(|_| "invalid AES-256 key")?;
        Ok(Self {
            key: LessSafeKey::new(key),
        })
    }

    pub fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, String> {
        let mut nonce = [0; 12];
        SystemRandom::new()
            .fill(&mut nonce)
            .map_err(|_| "secure random generator unavailable")?;
        let mut ciphertext = plaintext.to_vec();
        self.key
            .seal_in_place_append_tag(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(AAD),
                &mut ciphertext,
            )
            .map_err(|_| "state encryption failed")?;
        let mut out = Vec::with_capacity(PREFIX.len() + nonce.len() + ciphertext.len());
        out.extend_from_slice(PREFIX);
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ciphertext);
        Ok(out)
    }

    /// Returns the plaintext and whether it was already encrypted. Plain JSON is
    /// accepted only for one-time migration from earlier GaGaDown versions.
    pub fn decrypt_or_legacy(&self, stored: &[u8]) -> Result<(Vec<u8>, bool), String> {
        if !stored.starts_with(PREFIX) {
            return Ok((stored.to_vec(), false));
        }
        let nonce_start = PREFIX.len();
        let nonce_end = nonce_start + 12;
        if stored.len() < nonce_end + aead::AES_256_GCM.tag_len() {
            return Err("encrypted task state is truncated".into());
        }
        let nonce_bytes: [u8; 12] = stored[nonce_start..nonce_end].try_into().unwrap();
        let mut ciphertext = stored[nonce_end..].to_vec();
        let plaintext = self
            .key
            .open_in_place(Nonce::assume_unique_for_key(nonce_bytes), Aad::from(AAD), &mut ciphertext)
            .map_err(|_| "task state authentication failed; keychain key may be unavailable or data is damaged")?;
        Ok((plaintext.to_vec(), true))
    }
}

pub fn load_or_create_api_token() -> Result<String, String> {
    #[cfg(feature = "ci-test-keyring")]
    return Ok("0000000000000000000000000000000000000000000000000000000000000000".into());

    #[cfg(not(feature = "ci-test-keyring"))]
    {
        let entry = keyring::Entry::new(SERVICE, API_USER).map_err(|e| e.to_string())?;
        match entry.get_password() {
            Ok(token) if token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit()) => {
                Ok(token)
            }
            Ok(_) => Err("stored extension API token is invalid".into()),
            Err(keyring::Error::NoEntry) => {
                let mut token = [0; 32];
                SystemRandom::new()
                    .fill(&mut token)
                    .map_err(|_| "secure random generator unavailable")?;
                let token = hex::encode(token);
                entry.set_password(&token).map_err(|e| e.to_string())?;
                Ok(token)
            }
            Err(e) => Err(e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypts_and_authenticates_state() {
        let cipher = StateCipher::from_key_material(&[7; 32]).unwrap();
        let encrypted = cipher.encrypt(b"private task state").unwrap();
        assert!(encrypted.starts_with(PREFIX));
        assert!(!encrypted.windows(18).any(|w| w == b"private task state"));
        assert_eq!(
            cipher.decrypt_or_legacy(&encrypted).unwrap(),
            (b"private task state".to_vec(), true)
        );

        let mut tampered = encrypted;
        *tampered.last_mut().unwrap() ^= 1;
        assert!(cipher.decrypt_or_legacy(&tampered).is_err());
    }

    #[test]
    fn accepts_plain_json_for_migration() {
        let cipher = StateCipher::from_key_material(&[3; 32]).unwrap();
        assert_eq!(
            cipher.decrypt_or_legacy(b"{\"tasks\":[]}").unwrap(),
            (b"{\"tasks\":[]}".to_vec(), false)
        );
    }
}
