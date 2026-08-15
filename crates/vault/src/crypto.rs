//! Envelope encryption, and nothing else.
//!
//! No database, no filesystem, no clock — everything here is a pure function
//! of its arguments, which is what makes the guarantees testable.
//!
//! **The shape.** The root key never touches a secret. Each secret gets its
//! own randomly generated key; that key encrypts the value, and the root key
//! encrypts the key (hence the `wrapped_key` column). Rotating the root then
//! means re-encrypting a few dozen small keys rather than every value.
//!
//! **The binding.** Both layers are sealed with the secret's name as
//! associated data. Associated data isn't stored; it is supplied again at open
//! time and the tag only verifies if it matches — so a row renamed from
//! `github` to `anthropic` fails to open rather than quietly yielding the
//! wrong token.
//!
//! **The cipher.** XChaCha20-Poly1305. The 24-byte nonce is the reason:
//! nonces here are random, and at 24 bytes the chance of ever repeating one is
//! not a thing that needs managing.

use crate::VaultError;
use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng, Payload};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use zeroize::{Zeroize, Zeroizing};

/// XChaCha20-Poly1305: 32-byte key, 24-byte nonce.
const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 24;

/// Changing anything about how bytes are laid out means changing this, so old
/// ciphertexts fail loudly instead of decrypting into nonsense.
const DOMAIN: &[u8] = b"latoile/vault/v1";

/// Which envelope a tag belongs to, so a wrapped key and a value are not
/// interchangeable to the cipher.
#[derive(Clone, Copy)]
enum Layer {
    Wrap = 1,
    Value = 2,
}

/// Length-prefixed associated data: the name binds the ciphertext to its row.
fn associated_data(name: &str, layer: Layer) -> Vec<u8> {
    let mut out = Vec::with_capacity(DOMAIN.len() + name.len() + 8);
    out.extend_from_slice(DOMAIN);
    out.push(layer as u8);
    out.extend_from_slice(&(name.len() as u32).to_be_bytes());
    out.extend_from_slice(name.as_bytes());
    out
}

/// One sealed secret, as stored: two blobs, neither useful without the root
/// key, neither useful in another row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sealed {
    /// The secret's own key, encrypted under the root key.
    pub wrapped_key: Vec<u8>,
    /// The value, encrypted under the secret's own key. `nonce || ciphertext`.
    pub ciphertext: Vec<u8>,
}

/// The one key that isn't in the database. Zeroed on drop; never printed.
#[derive(Clone)]
pub struct RootKey([u8; KEY_LEN]);

impl Drop for RootKey {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl std::fmt::Debug for RootKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RootKey(…)")
    }
}

impl RootKey {
    /// A fresh key from the operating system's CSPRNG.
    pub fn generate() -> Self {
        Self(*random_key())
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, VaultError> {
        let bytes: [u8; KEY_LEN] = bytes
            .try_into()
            .map_err(|_| VaultError::KeyUnavailable("a root key is 32 bytes".into()))?;
        Ok(Self(bytes))
    }

    /// How the key is written down: base64, the one form that survives an
    /// environment variable and a copy-paste unchanged.
    pub fn decode(text: &str) -> Result<Self, VaultError> {
        use base64::Engine;
        let bytes = Zeroizing::new(
            base64::engine::general_purpose::STANDARD
                .decode(text.trim())
                .map_err(|_| VaultError::KeyUnavailable("a root key must be base64".into()))?,
        );
        Self::from_bytes(&bytes)
    }

    pub fn encode(&self) -> Zeroizing<String> {
        use base64::Engine;
        Zeroizing::new(base64::engine::general_purpose::STANDARD.encode(self.0))
    }

    /// Encrypt a value under a fresh key of its own.
    pub fn seal(&self, name: &str, plaintext: &[u8]) -> Result<Sealed, VaultError> {
        let secret_key = random_key();
        Ok(Sealed {
            wrapped_key: encrypt(
                Key::from_slice(&self.0),
                &associated_data(name, Layer::Wrap),
                &*secret_key,
            )?,
            ciphertext: encrypt(
                Key::from_slice(&*secret_key),
                &associated_data(name, Layer::Value),
                plaintext,
            )?,
        })
    }

    /// Decrypt, or fail. Every failure reads the same from the outside — a
    /// wrong key, a tampered byte, and a renamed row are all "this didn't
    /// verify". What went wrong is a detail an attacker would like.
    pub fn open(&self, name: &str, sealed: &Sealed) -> Result<Zeroizing<Vec<u8>>, VaultError> {
        let secret_key = Zeroizing::new(decrypt(
            Key::from_slice(&self.0),
            &associated_data(name, Layer::Wrap),
            &sealed.wrapped_key,
        )?);
        let key: [u8; KEY_LEN] = secret_key
            .as_slice()
            .try_into()
            .map_err(|_| VaultError::DecryptionFailed)?;
        Ok(Zeroizing::new(decrypt(
            Key::from_slice(&key),
            &associated_data(name, Layer::Value),
            &sealed.ciphertext,
        )?))
    }
}

/// A fresh 32 bytes from the operating system.
fn random_key() -> Zeroizing<[u8; KEY_LEN]> {
    let mut bytes = Zeroizing::new([0u8; KEY_LEN]);
    bytes.copy_from_slice(&XChaCha20Poly1305::generate_key(&mut OsRng));
    bytes
}

/// `nonce || ciphertext`, so a stored blob is self-describing.
fn encrypt(key: &Key, aad: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, VaultError> {
    let cipher = XChaCha20Poly1305::new(key);
    let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
    let mut out = nonce.to_vec();
    out.extend_from_slice(
        &cipher
            .encrypt(&nonce, Payload { msg: plaintext, aad })
            .map_err(|_| VaultError::KeyUnavailable("encryption failed".into()))?,
    );
    Ok(out)
}

fn decrypt(key: &Key, aad: &[u8], blob: &[u8]) -> Result<Vec<u8>, VaultError> {
    if blob.len() <= NONCE_LEN {
        return Err(VaultError::DecryptionFailed);
    }
    let (nonce, body) = blob.split_at(NONCE_LEN);
    XChaCha20Poly1305::new(key)
        .decrypt(XNonce::from_slice(nonce), Payload { msg: body, aad })
        .map_err(|_| VaultError::DecryptionFailed)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &[u8] = b"sk-ant-oat03-not-a-real-token";

    #[test]
    fn what_goes_in_comes_back_out() {
        let root = RootKey::generate();
        let sealed = root.seal("github", TOKEN).unwrap();
        assert_eq!(&*root.open("github", &sealed).unwrap(), TOKEN);
    }

    #[test]
    fn the_value_is_nowhere_in_the_stored_bytes() {
        let root = RootKey::generate();
        let sealed = root.seal("github", TOKEN).unwrap();
        for blob in [&sealed.wrapped_key, &sealed.ciphertext] {
            assert!(
                !blob.windows(TOKEN.len()).any(|w| w == TOKEN),
                "the plaintext survived into storage"
            );
        }
    }

    #[test]
    fn another_root_key_opens_nothing() {
        let sealed = RootKey::generate().seal("github", TOKEN).unwrap();
        assert!(RootKey::generate().open("github", &sealed).is_err());
    }

    /// The point of the associated data: a row moved to another name is not a
    /// credential for that name.
    #[test]
    fn a_secret_will_not_open_under_another_name() {
        let root = RootKey::generate();
        let sealed = root.seal("github", TOKEN).unwrap();
        assert!(root.open("anthropic", &sealed).is_err());
    }

    #[test]
    fn one_byte_changed_anywhere_fails() {
        let root = RootKey::generate();
        let sealed = root.seal("github", TOKEN).unwrap();
        for i in 0..sealed.ciphertext.len() {
            let mut bad = sealed.clone();
            bad.ciphertext[i] ^= 1;
            assert!(root.open("github", &bad).is_err(), "byte {i} of the value");
        }
        for i in 0..sealed.wrapped_key.len() {
            let mut bad = sealed.clone();
            bad.wrapped_key[i] ^= 1;
            assert!(root.open("github", &bad).is_err(), "byte {i} of the key");
        }
    }

    /// The two layers of one row do not interchange — that is what the layer
    /// byte in the associated data buys.
    #[test]
    fn the_two_layers_of_one_secret_do_not_interchange() {
        let root = RootKey::generate();
        let sealed = root.seal("github", TOKEN).unwrap();
        let swapped = Sealed {
            wrapped_key: sealed.ciphertext.clone(),
            ciphertext: sealed.wrapped_key.clone(),
        };
        assert!(root.open("github", &swapped).is_err());
    }

    #[test]
    fn the_same_value_sealed_twice_stores_differently() {
        let root = RootKey::generate();
        let once = root.seal("github", TOKEN).unwrap();
        let twice = root.seal("github", TOKEN).unwrap();
        assert_ne!(once.ciphertext, twice.ciphertext, "nonces must not repeat");
        assert_ne!(once.wrapped_key, twice.wrapped_key);
    }

    #[test]
    fn truncated_blobs_are_refused_rather_than_panicking() {
        let root = RootKey::generate();
        let sealed = root.seal("github", TOKEN).unwrap();
        for len in 0..=NONCE_LEN {
            let bad = Sealed {
                wrapped_key: sealed.wrapped_key[..len].to_vec(),
                ciphertext: sealed.ciphertext.clone(),
            };
            assert!(root.open("github", &bad).is_err());
        }
    }

    #[test]
    fn a_root_key_survives_being_written_down() {
        let root = RootKey::generate();
        let text = root.encode();
        let sealed = root.seal("github", TOKEN).unwrap();
        let read_back = RootKey::decode(&text).unwrap();
        assert_eq!(&*read_back.open("github", &sealed).unwrap(), TOKEN);
    }

    #[test]
    fn a_root_key_of_the_wrong_size_is_refused_at_the_door() {
        assert!(RootKey::decode("not base64 at all !!").is_err());
        assert!(RootKey::from_bytes(b"too short").is_err());
        assert!(RootKey::from_bytes(&[0u8; 64]).is_err());
    }

    #[test]
    fn a_key_never_prints_itself() {
        assert_eq!(format!("{:?}", RootKey::generate()), "RootKey(…)");
    }
}
