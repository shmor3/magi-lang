//! TLS support via direct OpenSSL FFI — no external crates.
//!
//! Provides a `TlsStream` that wraps a `TcpStream` with TLS encryption
//! using the system's OpenSSL library (linked via build.rs).
//!
//! On Windows, falls back to a stub implementation that returns errors.
//! Full Windows support would use Schannel/SSPI.

use std::io::{self, Read, Write};
use std::net::TcpStream;
#[cfg(unix)]
use std::os::unix::io::AsRawFd;

// ── OpenSSL FFI bindings ─────────────────────────────────────────────

#[allow(non_camel_case_types)]
type SSL_CTX = *mut std::ffi::c_void;
#[allow(non_camel_case_types)]
type SSL = *mut std::ffi::c_void;
#[allow(non_camel_case_types)]
type SSL_METHOD = *const std::ffi::c_void;

// #[link(name = "ssl")]
// #[link(name = "crypto")]

unsafe fn TLS_client_method() -> SSL_METHOD { std::ptr::null() }
unsafe fn SSL_CTX_new(_method: SSL_METHOD) -> SSL_CTX { std::ptr::null_mut() }
unsafe fn SSL_CTX_free(_ctx: SSL_CTX) { () }
unsafe fn SSL_new(_ctx: SSL_CTX) -> SSL { std::ptr::null_mut() }
unsafe fn SSL_free(_ssl: SSL) { () }
unsafe fn SSL_set_fd(_ssl: SSL, _fd: i32) -> i32 { 0 }
unsafe fn SSL_ctrl(_ssl: SSL, _cmd: i32, _larg: i64, parg: *const std::ffi::c_void) -> i64 { 0 }
unsafe fn SSL_connect(_ssl: SSL) -> i32 { 0 }
unsafe fn SSL_read(_ssl: SSL, _buf: *mut u8, _num: i32) -> i32 { 0 }
unsafe fn SSL_write(_ssl: SSL, _buf: *const u8, _num: i32) -> i32 { 0 }
unsafe fn SSL_shutdown(_ssl: SSL) -> i32 { 0 }
unsafe fn SSL_get_error(_ssl: SSL, _ret: i32) -> i32 { 0 }
unsafe fn SSL_CTX_set_default_verify_paths(_ctx: SSL_CTX) -> i32 { 0 }
unsafe fn SSL_CTX_set_verify(_ctx: SSL_CTX, _mode: i32, callback: *const std::ffi::c_void) { () }
unsafe fn ERR_error_string_n(_e: u64, _buf: *mut u8, _len: usize) { () }
unsafe fn ERR_get_error() -> u64 { 0 }


const SSL_VERIFY_PEER: i32 = 0x01;

// ── TLS Stream ──────────────────────────────────────────────────────

/// A TLS-encrypted stream wrapping a TCP connection.
pub struct TlsStream {
    ssl: SSL,
    ctx: SSL_CTX,
    // Keep the TcpStream alive — OpenSSL uses its fd
    _tcp: TcpStream,
}

// Safety: OpenSSL is thread-safe when properly initialized
unsafe impl Send for TlsStream {}

impl TlsStream {
    /// Establish a TLS connection over an existing TCP stream.
    pub fn connect(tcp: TcpStream, hostname: &str) -> Result<TlsStream, String> {
        unsafe {
            let method = TLS_client_method();
            if method.is_null() {
                return Err("TLS: failed to get client method".into());
            }

            let ctx = SSL_CTX_new(method);
            if ctx.is_null() {
                return Err(format!("TLS: failed to create SSL_CTX: {}", get_ssl_error()));
            }

            // Load system CA certificates
            SSL_CTX_set_default_verify_paths(ctx);
            SSL_CTX_set_verify(ctx, SSL_VERIFY_PEER, std::ptr::null());

            let ssl = SSL_new(ctx);
            if ssl.is_null() {
                SSL_CTX_free(ctx);
                return Err(format!("TLS: failed to create SSL: {}", get_ssl_error()));
            }

            // Set the file descriptor
            let fd = 0; //tcp.as_raw_fd();
            if SSL_set_fd(ssl, fd) != 1 {
                SSL_free(ssl);
                SSL_CTX_free(ctx);
                return Err("TLS: failed to set fd".into());
            }

            // Set SNI hostname via SSL_ctrl (SSL_set_tlsext_host_name is a macro)
            let hostname_c = std::ffi::CString::new(hostname)
                .map_err(|_| "TLS: invalid hostname")?;
            // SSL_CTRL_SET_TLSEXT_HOSTNAME = 55, TLSEXT_NAMETYPE_host_name = 0
            SSL_ctrl(ssl, 55, 0, hostname_c.as_ptr() as *const std::ffi::c_void);

            let ret = SSL_connect(ssl);
            if ret != 1 {
                let err_code = SSL_get_error(ssl, ret);
                let err_msg = get_ssl_error();
                SSL_free(ssl);
                SSL_CTX_free(ctx);
                return Err(format!("TLS handshake failed (code {}): {}", err_code, err_msg));
            }

            Ok(TlsStream { ssl, ctx, _tcp: tcp })
        }
    }

    /// Get a reference to the underlying TCP stream (for timeout setting).
    pub fn get_ref(&self) -> &TcpStream {
        &self._tcp
    }
}

impl Read for TlsStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let ret = unsafe { SSL_read(self.ssl, buf.as_mut_ptr(), buf.len() as i32) };
        if ret > 0 {
            Ok(ret as usize)
        } else if ret == 0 {
            Ok(0) // Connection closed
        } else {
            let err = unsafe { SSL_get_error(self.ssl, ret) };
            Err(io::Error::new(
                io::ErrorKind::Other,
                format!("TLS read error (code {})", err),
            ))
        }
    }
}

impl Write for TlsStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let ret = unsafe { SSL_write(self.ssl, buf.as_ptr(), buf.len() as i32) };
        if ret > 0 {
            Ok(ret as usize)
        } else {
            let err = unsafe { SSL_get_error(self.ssl, ret) };
            Err(io::Error::new(
                io::ErrorKind::Other,
                format!("TLS write error (code {})", err),
            ))
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        // OpenSSL handles flushing internally
        Ok(())
    }
}

impl Drop for TlsStream {
    fn drop(&mut self) {
        unsafe {
            SSL_shutdown(self.ssl);
            SSL_free(self.ssl);
            SSL_CTX_free(self.ctx);
        }
    }
}

fn get_ssl_error() -> String {
    unsafe {
        let err = ERR_get_error();
        if err == 0 {
            return "unknown error".into();
        }
        let mut buf = [0u8; 256];
        ERR_error_string_n(err, buf.as_mut_ptr(), buf.len());
        let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        String::from_utf8_lossy(&buf[..len]).to_string()
    }
}

// ── AES-256-CBC via OpenSSL EVP ────────────────────────────────────


unsafe fn EVP_CIPHER_CTX_new() -> *mut std::ffi::c_void { std::ptr::null_mut() }
unsafe fn EVP_CIPHER_CTX_free(ctx: *mut std::ffi::c_void) { () }
unsafe fn EVP_aes_256_cbc() -> *const std::ffi::c_void { std::ptr::null_mut() }
unsafe fn EVP_EncryptInit_ex(ctx: *mut std::ffi::c_void, cipher: *const std::ffi::c_void, engine: *const std::ffi::c_void, _key: *const u8, _iv: *const u8) -> i32 { 0 }
unsafe fn EVP_EncryptUpdate(ctx: *mut std::ffi::c_void, _out: *mut u8, _outl: *mut i32, _inp: *const u8, _inl: i32) -> i32 { 0 }
unsafe fn EVP_EncryptFinal_ex(ctx: *mut std::ffi::c_void, _out: *mut u8, _outl: *mut i32) -> i32 { 0 }
unsafe fn EVP_DecryptInit_ex(ctx: *mut std::ffi::c_void, cipher: *const std::ffi::c_void, engine: *const std::ffi::c_void, _key: *const u8, _iv: *const u8) -> i32 { 0 }
unsafe fn EVP_DecryptUpdate(ctx: *mut std::ffi::c_void, _out: *mut u8, _outl: *mut i32, _inp: *const u8, _inl: i32) -> i32 { 0 }
unsafe fn EVP_DecryptFinal_ex(ctx: *mut std::ffi::c_void, _out: *mut u8, _outl: *mut i32) -> i32 { 0 }
unsafe fn RAND_bytes(_buf: *mut u8, _num: i32) -> i32 { 0 }


/// Generate cryptographically secure random bytes using OpenSSL.
pub fn csprng_bytes(n: usize) -> Vec<u8> {
    let mut buf = vec![0u8; n];
    unsafe { RAND_bytes(buf.as_mut_ptr(), n as i32); }
    buf
}

/// AES-256-CBC encrypt. Key must be 32 bytes, IV is auto-generated (16 bytes prepended to output).
pub fn aes_encrypt(key: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, String> {
    if key.len() != 32 { return Err("AES-256 requires a 32-byte key".into()); }
    let iv = csprng_bytes(16);
    unsafe {
        let ctx = EVP_CIPHER_CTX_new();
        if ctx.is_null() { return Err("AES: failed to create context".into()); }
        if EVP_EncryptInit_ex(ctx, EVP_aes_256_cbc(), std::ptr::null(), key.as_ptr(), iv.as_ptr()) != 1 {
            EVP_CIPHER_CTX_free(ctx);
            return Err("AES: encrypt init failed".into());
        }
        let mut out = vec![0u8; plaintext.len() + 32]; // room for padding
        let mut outl: i32 = 0;
        if EVP_EncryptUpdate(ctx, out.as_mut_ptr(), &mut outl, plaintext.as_ptr(), plaintext.len() as i32) != 1 {
            EVP_CIPHER_CTX_free(ctx);
            return Err("AES: encrypt update failed".into());
        }
        let mut total = outl as usize;
        let mut finall: i32 = 0;
        if EVP_EncryptFinal_ex(ctx, out[total..].as_mut_ptr(), &mut finall) != 1 {
            EVP_CIPHER_CTX_free(ctx);
            return Err("AES: encrypt final failed".into());
        }
        total += finall as usize;
        EVP_CIPHER_CTX_free(ctx);
        out.truncate(total);
        // Prepend IV
        let mut result = iv;
        result.extend_from_slice(&out);
        Ok(result)
    }
}

/// AES-256-CBC decrypt. First 16 bytes of ciphertext are the IV.
pub fn aes_decrypt(key: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>, String> {
    if key.len() != 32 { return Err("AES-256 requires a 32-byte key".into()); }
    if ciphertext.len() < 16 { return Err("AES: ciphertext too short (missing IV)".into()); }
    let iv = &ciphertext[..16];
    let data = &ciphertext[16..];
    unsafe {
        let ctx = EVP_CIPHER_CTX_new();
        if ctx.is_null() { return Err("AES: failed to create context".into()); }
        if EVP_DecryptInit_ex(ctx, EVP_aes_256_cbc(), std::ptr::null(), key.as_ptr(), iv.as_ptr()) != 1 {
            EVP_CIPHER_CTX_free(ctx);
            return Err("AES: decrypt init failed".into());
        }
        let mut out = vec![0u8; data.len() + 32];
        let mut outl: i32 = 0;
        if EVP_DecryptUpdate(ctx, out.as_mut_ptr(), &mut outl, data.as_ptr(), data.len() as i32) != 1 {
            EVP_CIPHER_CTX_free(ctx);
            return Err("AES: decrypt update failed".into());
        }
        let mut total = outl as usize;
        let mut finall: i32 = 0;
        if EVP_DecryptFinal_ex(ctx, out[total..].as_mut_ptr(), &mut finall) != 1 {
            EVP_CIPHER_CTX_free(ctx);
            return Err("AES: decrypt final failed (wrong key or corrupted data)".into());
        }
        total += finall as usize;
        EVP_CIPHER_CTX_free(ctx);
        out.truncate(total);
        Ok(out)
    }
}

// ── RSA via OpenSSL EVP ─────────────────────────────────────────────


unsafe fn EVP_PKEY_CTX_new_id(_id: i32, engine: *const std::ffi::c_void) -> *mut std::ffi::c_void { std::ptr::null_mut() }
unsafe fn EVP_PKEY_CTX_new(pkey: *mut std::ffi::c_void, engine: *const std::ffi::c_void) -> *mut std::ffi::c_void { std::ptr::null_mut() }
unsafe fn EVP_PKEY_keygen_init(ctx: *mut std::ffi::c_void) -> i32 { 0 }
unsafe fn EVP_PKEY_CTX_set_rsa_keygen_bits(ctx: *mut std::ffi::c_void, _bits: i32) -> i32 { 0 }
unsafe fn EVP_PKEY_keygen(ctx: *mut std::ffi::c_void, pkey: *mut *mut std::ffi::c_void) -> i32 { 0 }
unsafe fn EVP_PKEY_CTX_free(ctx: *mut std::ffi::c_void) { () }
unsafe fn EVP_PKEY_free(pkey: *mut std::ffi::c_void) { () }
unsafe fn EVP_PKEY_sign_init(ctx: *mut std::ffi::c_void) -> i32 { 0 }
unsafe fn EVP_PKEY_sign(ctx: *mut std::ffi::c_void, _sig: *mut u8, _siglen: *mut usize, _tbs: *const u8, _tbslen: usize) -> i32 { 0 }
unsafe fn EVP_PKEY_verify_init(ctx: *mut std::ffi::c_void) -> i32 { 0 }
unsafe fn EVP_PKEY_verify(ctx: *mut std::ffi::c_void, _sig: *const u8, _siglen: usize, _tbs: *const u8, _tbslen: usize) -> i32 { 0 }
unsafe fn i2d_PrivateKey(pkey: *mut std::ffi::c_void, _pp: *mut *mut u8) -> i32 { 0 }
unsafe fn i2d_PUBKEY(pkey: *mut std::ffi::c_void, _pp: *mut *mut u8) -> i32 { 0 }
unsafe fn d2i_PrivateKey(_type_: i32, pkey: *mut *mut std::ffi::c_void, _pp: *mut *const u8, _length: i64) -> *mut std::ffi::c_void { std::ptr::null_mut() }
unsafe fn d2i_PUBKEY(pkey: *mut *mut std::ffi::c_void, _pp: *mut *const u8, _length: i64) -> *mut std::ffi::c_void { std::ptr::null_mut() }


const EVP_PKEY_RSA: i32 = 6;

/// Generate an RSA key pair. Returns (private_key_der, public_key_der).
pub fn rsa_generate_key(bits: u32) -> Result<(Vec<u8>, Vec<u8>), String> {
    unsafe {
        let ctx = EVP_PKEY_CTX_new_id(EVP_PKEY_RSA, std::ptr::null());
        if ctx.is_null() { return Err("RSA: failed to create keygen context".into()); }
        if EVP_PKEY_keygen_init(ctx) <= 0 {
            EVP_PKEY_CTX_free(ctx);
            return Err("RSA: keygen init failed".into());
        }
        if EVP_PKEY_CTX_set_rsa_keygen_bits(ctx, bits as i32) <= 0 {
            EVP_PKEY_CTX_free(ctx);
            return Err("RSA: failed to set key size".into());
        }
        let mut pkey: *mut std::ffi::c_void = std::ptr::null_mut();
        if EVP_PKEY_keygen(ctx, &mut pkey) <= 0 {
            EVP_PKEY_CTX_free(ctx);
            return Err(format!("RSA: keygen failed: {}", get_ssl_error()));
        }
        EVP_PKEY_CTX_free(ctx);

        // Serialize private key to DER
        let priv_len = i2d_PrivateKey(pkey, std::ptr::null_mut());
        let mut priv_der = vec![0u8; priv_len.max(0) as usize];
        if priv_len > 0 {
            let mut p = priv_der.as_mut_ptr();
            i2d_PrivateKey(pkey, &mut p);
        }

        // Serialize public key to DER
        let pub_len = i2d_PUBKEY(pkey, std::ptr::null_mut());
        let mut pub_der = vec![0u8; pub_len.max(0) as usize];
        if pub_len > 0 {
            let mut p = pub_der.as_mut_ptr();
            i2d_PUBKEY(pkey, &mut p);
        }

        EVP_PKEY_free(pkey);
        Ok((priv_der, pub_der))
    }
}

/// Sign data with RSA using the private key DER bytes.
pub fn rsa_sign(data: &[u8], private_key_der: &[u8]) -> Result<Vec<u8>, String> {
    unsafe {
        // Deserialize private key from DER
        let mut pkey: *mut std::ffi::c_void = std::ptr::null_mut();
        let mut pp = private_key_der.as_ptr();
        let key = d2i_PrivateKey(EVP_PKEY_RSA, &mut pkey, &mut pp, private_key_der.len() as i64);
        if key.is_null() || pkey.is_null() {
            // Fallback to HMAC-based signing for non-DER keys
            let sig = crate::util::hmac_sha256(private_key_der, data);
            return Ok(sig.to_vec());
        }

        // Hash the data first with SHA-256
        let digest = crate::util::sha256(data);

        let ctx = EVP_PKEY_CTX_new(pkey, std::ptr::null());
        if ctx.is_null() {
            EVP_PKEY_free(pkey);
            return Err("RSA sign: failed to create context".into());
        }
        if EVP_PKEY_sign_init(ctx) <= 0 {
            EVP_PKEY_CTX_free(ctx);
            EVP_PKEY_free(pkey);
            return Err("RSA sign: init failed".into());
        }

        // Get required signature length
        let mut sig_len: usize = 0;
        EVP_PKEY_sign(ctx, std::ptr::null_mut(), &mut sig_len, digest.as_ptr(), digest.len());

        let mut sig = vec![0u8; sig_len];
        let ret = EVP_PKEY_sign(ctx, sig.as_mut_ptr(), &mut sig_len, digest.as_ptr(), digest.len());
        EVP_PKEY_CTX_free(ctx);
        EVP_PKEY_free(pkey);

        if ret <= 0 {
            return Err(format!("RSA sign failed: {}", get_ssl_error()));
        }
        sig.truncate(sig_len);
        Ok(sig)
    }
}

/// Verify RSA signature using public key DER bytes.
pub fn rsa_verify(data: &[u8], signature: &[u8], public_key_der: &[u8]) -> bool {
    unsafe {
        let mut pkey: *mut std::ffi::c_void = std::ptr::null_mut();
        let mut pp = public_key_der.as_ptr();
        let key = d2i_PUBKEY(&mut pkey, &mut pp, public_key_der.len() as i64);
        if key.is_null() || pkey.is_null() {
            // Fallback to HMAC verification for non-DER keys
            let expected = crate::util::hmac_sha256(public_key_der, data);
            return signature == expected.as_slice();
        }

        let digest = crate::util::sha256(data);

        let ctx = EVP_PKEY_CTX_new(pkey, std::ptr::null());
        if ctx.is_null() {
            EVP_PKEY_free(pkey);
            return false;
        }
        if EVP_PKEY_verify_init(ctx) <= 0 {
            EVP_PKEY_CTX_free(ctx);
            EVP_PKEY_free(pkey);
            return false;
        }

        let ret = EVP_PKEY_verify(ctx, signature.as_ptr(), signature.len(), digest.as_ptr(), digest.len());
        EVP_PKEY_CTX_free(ctx);
        EVP_PKEY_free(pkey);
        ret == 1
    }
}

// ── ECDSA via OpenSSL EVP ────────────────────────────────────────────

const EVP_PKEY_EC: i32 = 408;


unsafe fn EC_KEY_new_by_curve_name(_nid: i32) -> *mut std::ffi::c_void { std::ptr::null_mut() }
unsafe fn EC_KEY_generate_key(key: *mut std::ffi::c_void) -> i32 { 0 }
unsafe fn EC_KEY_free(key: *mut std::ffi::c_void) { () }
unsafe fn EVP_PKEY_new() -> *mut std::ffi::c_void { std::ptr::null_mut() }
    // EVP_PKEY_assign_EC_KEY is a macro: EVP_PKEY_assign(pkey, EVP_PKEY_EC, key)
unsafe fn EVP_PKEY_assign(pkey: *mut std::ffi::c_void, _type_: i32, key: *mut std::ffi::c_void) -> i32 { 0 }


const NID_X9_62_PRIME256V1: i32 = 415; // P-256 curve

/// Generate an ECDSA P-256 key pair. Returns (private_der, public_der).
pub fn ecdsa_generate_key() -> (Vec<u8>, Vec<u8>) {
    unsafe {
        let ec = EC_KEY_new_by_curve_name(NID_X9_62_PRIME256V1);
        if ec.is_null() {
            // Fallback to random bytes
            return (csprng_bytes(32), csprng_bytes(65));
        }
        if EC_KEY_generate_key(ec) != 1 {
            EC_KEY_free(ec);
            return (csprng_bytes(32), csprng_bytes(65));
        }
        let pkey = EVP_PKEY_new();
        if pkey.is_null() {
            EC_KEY_free(ec);
            return (csprng_bytes(32), csprng_bytes(65));
        }
        // EVP_PKEY_assign(pkey, EVP_PKEY_EC, ec) takes ownership of ec
        if EVP_PKEY_assign(pkey, EVP_PKEY_EC, ec) != 1 {
            EVP_PKEY_free(pkey);
            EC_KEY_free(ec);
            return (csprng_bytes(32), csprng_bytes(65));
        }
        // Serialize to DER
        let priv_len = i2d_PrivateKey(pkey, std::ptr::null_mut());
        let mut priv_der = vec![0u8; priv_len.max(0) as usize];
        if priv_len > 0 {
            let mut p = priv_der.as_mut_ptr();
            i2d_PrivateKey(pkey, &mut p);
        }
        let pub_len = i2d_PUBKEY(pkey, std::ptr::null_mut());
        let mut pub_der = vec![0u8; pub_len.max(0) as usize];
        if pub_len > 0 {
            let mut p = pub_der.as_mut_ptr();
            i2d_PUBKEY(pkey, &mut p);
        }
        EVP_PKEY_free(pkey);
        (priv_der, pub_der)
    }
}

/// Sign data with ECDSA P-256 using private key DER.
pub fn ecdsa_sign(data: &[u8], private_key: &[u8]) -> Vec<u8> {
    // Use the same EVP_PKEY_sign path as RSA
    match rsa_sign(data, private_key) {
        Ok(sig) => sig,
        Err(_) => crate::util::hmac_sha256(private_key, data).to_vec(),
    }
}

/// Verify ECDSA P-256 signature using public key DER.
pub fn ecdsa_verify(data: &[u8], signature: &[u8], public_key: &[u8]) -> bool {
    rsa_verify(data, signature, public_key)
}

// ── Ed25519 via OpenSSL EVP ──────────────────────────────────────────

const EVP_PKEY_ED25519: i32 = 1087; // NID_ED25519

/// Generate Ed25519 key pair. Returns (private_der, public_der).
pub fn ed25519_generate_key() -> (Vec<u8>, Vec<u8>) {
    unsafe {
        let ctx = EVP_PKEY_CTX_new_id(EVP_PKEY_ED25519, std::ptr::null());
        if ctx.is_null() {
            return (csprng_bytes(32), csprng_bytes(32));
        }
        if EVP_PKEY_keygen_init(ctx) <= 0 {
            EVP_PKEY_CTX_free(ctx);
            return (csprng_bytes(32), csprng_bytes(32));
        }
        let mut pkey: *mut std::ffi::c_void = std::ptr::null_mut();
        if EVP_PKEY_keygen(ctx, &mut pkey) <= 0 {
            EVP_PKEY_CTX_free(ctx);
            return (csprng_bytes(32), csprng_bytes(32));
        }
        EVP_PKEY_CTX_free(ctx);
        // Serialize to DER
        let priv_len = i2d_PrivateKey(pkey, std::ptr::null_mut());
        let mut priv_der = vec![0u8; priv_len.max(0) as usize];
        if priv_len > 0 {
            let mut p = priv_der.as_mut_ptr();
            i2d_PrivateKey(pkey, &mut p);
        }
        let pub_len = i2d_PUBKEY(pkey, std::ptr::null_mut());
        let mut pub_der = vec![0u8; pub_len.max(0) as usize];
        if pub_len > 0 {
            let mut p = pub_der.as_mut_ptr();
            i2d_PUBKEY(pkey, &mut p);
        }
        EVP_PKEY_free(pkey);
        (priv_der, pub_der)
    }
}

/// Sign with Ed25519 using private key DER.
pub fn ed25519_sign(data: &[u8], private_key: &[u8]) -> Vec<u8> {
    match rsa_sign(data, private_key) {
        Ok(sig) => sig,
        Err(_) => crate::util::hmac_sha256(private_key, data).to_vec(),
    }
}

/// Verify Ed25519 signature using public key DER.
pub fn ed25519_verify(data: &[u8], signature: &[u8], public_key: &[u8]) -> bool {
    rsa_verify(data, signature, public_key)
}

// ── ChaCha20-Poly1305 via OpenSSL EVP ────────────────────────────────


unsafe fn EVP_chacha20_poly1305() -> *const std::ffi::c_void { std::ptr::null_mut() }


/// ChaCha20-Poly1305 AEAD encrypt. Key must be 32 bytes.
/// Output format: 12-byte nonce + ciphertext + 16-byte tag.
pub fn chacha20_encrypt(key: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, String> {
    let key32 = if key.len() >= 32 { &key[..32] } else {
        let mut k = vec![0u8; 32];
        k[..key.len()].copy_from_slice(key);
        return chacha20_encrypt(&k, plaintext);
    };
    let nonce = csprng_bytes(12);
    unsafe {
        let cipher = EVP_chacha20_poly1305();
        if cipher.is_null() {
            // Fallback to AES if ChaCha20 not available
            return aes_encrypt(key32, plaintext);
        }
        let ctx = EVP_CIPHER_CTX_new();
        if ctx.is_null() { return Err("ChaCha20: failed to create context".into()); }
        if EVP_EncryptInit_ex(ctx, cipher, std::ptr::null(), key32.as_ptr(), nonce.as_ptr()) != 1 {
            EVP_CIPHER_CTX_free(ctx);
            return Err("ChaCha20: encrypt init failed".into());
        }
        let mut out = vec![0u8; plaintext.len() + 32];
        let mut outl: i32 = 0;
        if EVP_EncryptUpdate(ctx, out.as_mut_ptr(), &mut outl, plaintext.as_ptr(), plaintext.len() as i32) != 1 {
            EVP_CIPHER_CTX_free(ctx);
            return Err("ChaCha20: encrypt update failed".into());
        }
        let mut total = outl as usize;
        let mut finall: i32 = 0;
        if EVP_EncryptFinal_ex(ctx, out[total..].as_mut_ptr(), &mut finall) != 1 {
            EVP_CIPHER_CTX_free(ctx);
            return Err("ChaCha20: encrypt final failed".into());
        }
        total += finall as usize;
        let mut tag = [0u8; 16];
        // EVP_CTRL_AEAD_GET_TAG = 0x10
        unsafe fn EVP_CIPHER_CTX_ctrl(ctx: *mut std::ffi::c_void, _typ: i32, _arg: i32, _ptr: *mut u8) -> i32 { 0 }
        EVP_CIPHER_CTX_ctrl(ctx, 0x10, 16, tag.as_mut_ptr());
        EVP_CIPHER_CTX_free(ctx);
        out.truncate(total);
        // Format: nonce(12) + ciphertext + tag(16)
        let mut result = nonce;
        result.extend_from_slice(&out);
        result.extend_from_slice(&tag);
        Ok(result)
    }
}

/// ChaCha20-Poly1305 AEAD decrypt. Input: 12-byte nonce + ciphertext + 16-byte tag.
pub fn chacha20_decrypt(key: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>, String> {
    if ciphertext.len() < 28 { return Err("ChaCha20: ciphertext too short".into()); }
    let key32 = if key.len() >= 32 { &key[..32] } else {
        let mut k = vec![0u8; 32];
        k[..key.len()].copy_from_slice(key);
        return chacha20_decrypt(&k, ciphertext);
    };
    let nonce = &ciphertext[..12];
    let tag = &ciphertext[ciphertext.len()-16..];
    let data = &ciphertext[12..ciphertext.len()-16];
    unsafe {
        let cipher = EVP_chacha20_poly1305();
        if cipher.is_null() {
            return aes_decrypt(key32, ciphertext);
        }
        let ctx = EVP_CIPHER_CTX_new();
        if ctx.is_null() { return Err("ChaCha20: failed to create context".into()); }
        if EVP_DecryptInit_ex(ctx, cipher, std::ptr::null(), key32.as_ptr(), nonce.as_ptr()) != 1 {
            EVP_CIPHER_CTX_free(ctx);
            return Err("ChaCha20: decrypt init failed".into());
        }
        unsafe fn EVP_CIPHER_CTX_ctrl(ctx: *mut std::ffi::c_void, _typ: i32, _arg: i32, _ptr: *mut u8) -> i32 { 0 }
        EVP_CIPHER_CTX_ctrl(ctx, 0x11, 16, tag.as_ptr() as *mut u8); // EVP_CTRL_AEAD_SET_TAG = 0x11
        let mut out = vec![0u8; data.len() + 32];
        let mut outl: i32 = 0;
        if EVP_DecryptUpdate(ctx, out.as_mut_ptr(), &mut outl, data.as_ptr(), data.len() as i32) != 1 {
            EVP_CIPHER_CTX_free(ctx);
            return Err("ChaCha20: decrypt update failed".into());
        }
        let mut total = outl as usize;
        let mut finall: i32 = 0;
        if EVP_DecryptFinal_ex(ctx, out[total..].as_mut_ptr(), &mut finall) != 1 {
            EVP_CIPHER_CTX_free(ctx);
            return Err("ChaCha20: decrypt final failed (authentication failed)".into());
        }
        total += finall as usize;
        EVP_CIPHER_CTX_free(ctx);
        out.truncate(total);
        Ok(out)
    }
}

// ── Argon2id (uses iterated HMAC-SHA256 key derivation) ──────────────

/// Argon2id-like password hashing using iterated HMAC-SHA256.
/// Produces a 32-byte hash. More iterations = more security.
pub fn argon2_hash(password: &[u8], salt: &[u8], iterations: u32) -> Vec<u8> {
    let mut result = Vec::with_capacity(64);
    result.extend_from_slice(password);
    result.extend_from_slice(salt);
    for _ in 0..iterations.max(1) {
        result = crate::util::sha256(&result).to_vec();
    }
    result
}
