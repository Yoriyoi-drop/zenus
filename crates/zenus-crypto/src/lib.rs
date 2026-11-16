#![no_std]

// The host test harness needs std; the bare-metal build uses alloc.
extern crate alloc;

#[cfg(test)]
extern crate std;

use alloc::vec::Vec;
use core::fmt;

/// Stub result type for crypto operations.
pub type CryptoResult<T> = Result<T, CryptoError>;

/// Errors from crypto operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CryptoError {
    NotImplemented,
    InvalidInput,
    AllocationFailed,
    UnsupportedAlgorithm,
}

impl fmt::Display for CryptoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CryptoError::NotImplemented => write!(f, "crypto: operation not implemented"),
            CryptoError::InvalidInput => write!(f, "crypto: invalid input"),
            CryptoError::AllocationFailed => write!(f, "crypto: allocation failed"),
            CryptoError::UnsupportedAlgorithm => {
                write!(f, "crypto: unsupported algorithm")
            }
        }
    }
}

/// Hash algorithm advertised by the stub.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum HashAlgo {
    Sha256 = 0,
    Sha512 = 1,
}

impl HashAlgo {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(HashAlgo::Sha256),
            1 => Some(HashAlgo::Sha512),
            _ => None,
        }
    }

    pub fn output_size(self) -> usize {
        match self {
            HashAlgo::Sha256 => 32,
            HashAlgo::Sha512 => 64,
        }
    }
}

/// Symmetric cipher advertised by the stub.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CipherAlgo {
    Aes128 = 0,
    Aes256 = 1,
    ChaCha20 = 2,
}

impl CipherAlgo {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(CipherAlgo::Aes128),
            1 => Some(CipherAlgo::Aes256),
            2 => Some(CipherAlgo::ChaCha20),
            _ => None,
        }
    }

    pub fn key_size(self) -> usize {
        match self {
            CipherAlgo::Aes128 => 16,
            CipherAlgo::Aes256 => 32,
            CipherAlgo::ChaCha20 => 32,
        }
    }
}

/// Hash a byte slice. Stub: returns NotImplemented.
pub fn hash(data: &[u8], algo: HashAlgo) -> CryptoResult<Vec<u8>> {
    let _ = data;
    let _ = algo;
    Err(CryptoError::NotImplemented)
}

/// Encrypt a byte slice. Stub: returns NotImplemented.
pub fn encrypt(
    plaintext: &[u8],
    key: &[u8],
    algo: CipherAlgo,
) -> CryptoResult<Vec<u8>> {
    let _ = plaintext;
    let _ = key;
    let _ = algo;
    Err(CryptoError::NotImplemented)
}

/// Decrypt a byte slice. Stub: returns NotImplemented.
pub fn decrypt(
    ciphertext: &[u8],
    key: &[u8],
    algo: CipherAlgo,
) -> CryptoResult<Vec<u8>> {
    let _ = ciphertext;
    let _ = key;
    let _ = algo;
    Err(CryptoError::NotImplemented)
}

/// Compute an HMAC. Stub: returns NotImplemented.
pub fn hmac(
    key: &[u8],
    data: &[u8],
    algo: HashAlgo,
) -> CryptoResult<Vec<u8>> {
    let _ = key;
    let _ = data;
    let _ = algo;
    Err(CryptoError::NotImplemented)
}

/// Generate random bytes. Stub: returns NotImplemented.
pub fn fill_random(buf: &mut [u8]) -> CryptoResult<()> {
    let _ = buf;
    Err(CryptoError::NotImplemented)
}

#[cfg(test)]
mod host_tests {
    use super::*;

    #[test]
    fn hash_returns_not_implemented() {
        match hash(b"hello", HashAlgo::Sha256) {
            Err(CryptoError::NotImplemented) => {}
            other => panic!("expected NotImplemented, got {:?}", other),
        }
    }

    #[test]
    fn encrypt_returns_not_implemented() {
        match encrypt(b"plaintext", b"key0123456789012", CipherAlgo::Aes128) {
            Err(CryptoError::NotImplemented) => {}
            other => panic!("expected NotImplemented, got {:?}", other),
        }
    }

    #[test]
    fn decrypt_returns_not_implemented() {
        match decrypt(b"ciphertext", b"key0123456789012", CipherAlgo::Aes128) {
            Err(CryptoError::NotImplemented) => {}
            other => panic!("expected NotImplemented, got {:?}", other),
        }
    }

    #[test]
    fn hmac_returns_not_implemented() {
        match hmac(b"key", b"data", HashAlgo::Sha256) {
            Err(CryptoError::NotImplemented) => {}
            other => panic!("expected NotImplemented, got {:?}", other),
        }
    }

    #[test]
    fn fill_random_returns_not_implemented() {
        let mut buf = [0u8; 16];
        match fill_random(&mut buf) {
            Err(CryptoError::NotImplemented) => {}
            other => panic!("expected NotImplemented, got {:?}", other),
        }
    }

    #[test]
    fn hash_output_sizes() {
        assert_eq!(HashAlgo::Sha256.output_size(), 32);
        assert_eq!(HashAlgo::Sha512.output_size(), 64);
    }

    #[test]
    fn cipher_key_sizes() {
        assert_eq!(CipherAlgo::Aes128.key_size(), 16);
        assert_eq!(CipherAlgo::Aes256.key_size(), 32);
        assert_eq!(CipherAlgo::ChaCha20.key_size(), 32);
    }
}
