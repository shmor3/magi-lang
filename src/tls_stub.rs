//! Windows stub implementation for TLS

use std::io::{self, Read, Write};
use std::net::TcpStream;

pub struct TlsStream {
    _tcp: TcpStream,
}

impl TlsStream {
    pub fn connect(_tcp: TcpStream, _hostname: &str) -> Result<TlsStream, String> {
        Err("TLS is not supported on this platform (stub implementation)".to_string())
    }

    pub fn get_ref(&self) -> &TcpStream {
        &self._tcp
    }
}

impl Read for TlsStream {
    fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "TLS not supported"))
    }
}

impl Write for TlsStream {
    fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "TLS not supported"))
    }
    fn flush(&mut self) -> io::Result<()> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "TLS not supported"))
    }
}

pub fn csprng_bytes(n: usize) -> Vec<u8> {
    vec![0; n]
}

pub fn aes_encrypt(_key: &[u8], _plaintext: &[u8]) -> Result<Vec<u8>, String> {
    Err("AES not supported".to_string())
}

pub fn aes_decrypt(_key: &[u8], _ciphertext: &[u8]) -> Result<Vec<u8>, String> {
    Err("AES not supported".to_string())
}

pub fn rsa_generate_key(_bits: u32) -> Result<(Vec<u8>, Vec<u8>), String> {
    Err("RSA not supported".to_string())
}

pub fn rsa_sign(_data: &[u8], _private_key_der: &[u8]) -> Result<Vec<u8>, String> {
    Err("RSA not supported".to_string())
}

pub fn rsa_verify(_data: &[u8], _signature: &[u8], _public_key_der: &[u8]) -> bool {
    false
}

pub fn ecdsa_generate_key() -> (Vec<u8>, Vec<u8>) {
    (vec![], vec![])
}

pub fn ecdsa_sign(_data: &[u8], _private_key: &[u8]) -> Vec<u8> {
    vec![]
}

pub fn ecdsa_verify(_data: &[u8], _signature: &[u8], _public_key: &[u8]) -> bool {
    false
}

pub fn ed25519_generate_key() -> (Vec<u8>, Vec<u8>) {
    (vec![], vec![])
}

pub fn ed25519_sign(_data: &[u8], _private_key: &[u8]) -> Vec<u8> {
    vec![]
}

pub fn ed25519_verify(_data: &[u8], _signature: &[u8], _public_key: &[u8]) -> bool {
    false
}

pub fn chacha20_encrypt(_key: &[u8], _plaintext: &[u8]) -> Result<Vec<u8>, String> {
    Err("ChaCha20 not supported".to_string())
}

pub fn chacha20_decrypt(_key: &[u8], _ciphertext: &[u8]) -> Result<Vec<u8>, String> {
    Err("ChaCha20 not supported".to_string())
}

pub fn argon2_hash(_password: &[u8], _salt: &[u8], _iterations: u32) -> Vec<u8> {
    vec![]
}
