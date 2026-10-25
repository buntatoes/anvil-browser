// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! TLS for one already-connected TCP socket.
//!
//! Windows uses Schannel (`secur32`). Certificate verification is on:
//! `SCH_CRED_AUTO_CRED_VALIDATION` asks Schannel to validate the server
//! certificate. This file does not set manual validation and does not disable
//! the check. Linux uses OpenSSL `libssl` and `libcrypto`. Verification is on:
//! `SSL_VERIFY_PEER`, the default CA paths, a hostname check, and
//! `SSL_get_verify_result` must be `X509_V_OK`. The minimum protocol requested
//! on Linux is TLS 1.2.
//!
//! On Linux, `TlsSession` offers `h2` and `http/1.1` through ALPN and reads
//! and writes in turns, which HTTP/2 needs. Windows does not send ALPN yet,
//! so its connections stay on HTTP/1.1.
//!
//! [`https_exchange`] is called from `http_exchange` after `TcpStream::connect`.
//! It writes one HTTP/1.1 request and reads until the peer closes, the read
//! times out with bytes already buffered, or the raw buffer would pass
//! `max_response`. There is no session resumption and no connection pool.
//! Renegotiation on Windows stops the read instead of continuing the handshake.
//! Other operating systems have no implementation. The caller returns
//! `UnsupportedScheme` for `https` there.
//!
//! `build.rs` only searches for the Linux libraries. A missing library warns
//! and does not fail that script. The link names are declared in this file.

#[cfg(windows)]
mod imp {
    //! Schannel client for one socket.
    //!
    //! Certificate verification is on through `SCH_CRED_AUTO_CRED_VALIDATION`.
    //! No client certificate is sent. The decrypted body is capped by `max_response`.

    use std::ffi::c_void;
    use std::io::{Error, ErrorKind, Read, Write};
    use std::net::TcpStream;
    use std::ptr::{null, null_mut};

    use crate::NetError;

    const SECPKG_CRED_OUTBOUND: u32 = 0x0000_0002;
    const SCHANNEL_CRED_VERSION: u32 = 4;
    // Do not send a client certificate.
    const SCH_CRED_NO_DEFAULT_CREDS: u32 = 0x0000_0010;
    // Certificate verification is on. Schannel validates the server certificate.
    const SCH_CRED_AUTO_CRED_VALIDATION: u32 = 0x0000_0020;
    // Refuse known weak algorithms. Protocol versions are left at the Schannel default.
    const SCH_USE_STRONG_CRYPTO: u32 = 0x0040_0000;

    const ISC_REQ_REPLAY_DETECT: u32 = 0x0000_0004;
    const ISC_REQ_SEQUENCE_DETECT: u32 = 0x0000_0008;
    const ISC_REQ_CONFIDENTIALITY: u32 = 0x0000_0010;
    const ISC_REQ_USE_SUPPLIED_CREDS: u32 = 0x0000_0080;
    const ISC_REQ_ALLOCATE_MEMORY: u32 = 0x0000_0100;
    const ISC_REQ_EXTENDED_ERROR: u32 = 0x0000_4000;
    const ISC_REQ_STREAM: u32 = 0x0000_8000;

    const SECURITY_NATIVE_DREP: u32 = 0x0000_0010;

    const SECBUFFER_VERSION: u32 = 0;
    const SECBUFFER_EMPTY: u32 = 0;
    const SECBUFFER_DATA: u32 = 1;
    const SECBUFFER_TOKEN: u32 = 2;
    const SECBUFFER_EXTRA: u32 = 5;
    const SECBUFFER_STREAM_TRAILER: u32 = 6;
    const SECBUFFER_STREAM_HEADER: u32 = 7;

    const SECPKG_ATTR_STREAM_SIZES: u32 = 4;

    const SEC_E_OK: i32 = 0;
    const SEC_I_CONTINUE_NEEDED: i32 = 0x0009_0312;
    const SEC_I_CONTEXT_EXPIRED: i32 = 0x0009_0317;
    const SEC_I_RENEGOTIATE: i32 = 0x0009_0321;
    const SEC_E_INCOMPLETE_MESSAGE: i32 = 0x8009_0318_u32 as i32;

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct SecHandle {
        dw_lower: usize,
        dw_upper: usize,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct SecurityInteger {
        low_part: u32,
        high_part: i32,
    }

    #[repr(C)]
    struct SchannelCred {
        dw_version: u32,
        c_creds: u32,
        pa_cred: *const *const c_void,
        h_root_store: *mut c_void,
        c_mappers: u32,
        aph_mappers: *const *mut c_void,
        c_supported_algs: u32,
        palg_supported_algs: *const u32,
        grbit_enabled_protocols: u32,
        dw_minimum_cipher_strength: u32,
        dw_maximum_cipher_strength: u32,
        dw_session_lifespan: u32,
        dw_flags: u32,
        dw_cred_format: u32,
    }

    #[repr(C)]
    struct SecBuffer {
        cb_buffer: u32,
        buffer_type: u32,
        pv_buffer: *mut c_void,
    }

    #[repr(C)]
    struct SecBufferDesc {
        ul_version: u32,
        c_buffers: u32,
        p_buffers: *mut SecBuffer,
    }

    #[repr(C)]
    #[derive(Default)]
    struct SecPkgContextStreamSizes {
        cb_header: u32,
        cb_trailer: u32,
        cb_maximum_message: u32,
        c_buffers: u32,
        cb_block_size: u32,
    }

    // Schannel entry points from secur32. Certificate checks stay on.
    // AcquireCredentialsHandleW loads the credential. InitializeSecurityContextW
    // runs the handshake. EncryptMessage and DecryptMessage wrap application
    // data. The three Free/Delete calls run from TlsGuards on drop.
    #[link(name = "secur32")]
    unsafe extern "system" {
        fn AcquireCredentialsHandleW(
            psz_principal: *const u16,
            psz_package: *const u16,
            f_credential_use: u32,
            pv_logon_id: *const c_void,
            p_auth_data: *const c_void,
            p_get_key_fn: *const c_void,
            pv_get_key_argument: *const c_void,
            ph_credential: *mut SecHandle,
            pts_expiry: *mut SecurityInteger,
        ) -> i32;

        fn InitializeSecurityContextW(
            ph_credential: *mut SecHandle,
            ph_context: *mut SecHandle,
            psz_target_name: *const u16,
            f_context_req: u32,
            reserved1: u32,
            target_data_rep: u32,
            p_input: *mut SecBufferDesc,
            reserved2: u32,
            ph_new_context: *mut SecHandle,
            p_output: *mut SecBufferDesc,
            pf_context_attr: *mut u32,
            pts_expiry: *mut SecurityInteger,
        ) -> i32;

        fn QueryContextAttributesW(
            ph_context: *mut SecHandle,
            ul_attribute: u32,
            p_buffer: *mut c_void,
        ) -> i32;

        fn EncryptMessage(
            ph_context: *mut SecHandle,
            f_qop: u32,
            p_message: *mut SecBufferDesc,
            message_seq_no: u32,
        ) -> i32;

        fn DecryptMessage(
            ph_context: *mut SecHandle,
            p_message: *mut SecBufferDesc,
            message_seq_no: u32,
            pf_qop: *mut u32,
        ) -> i32;

        fn FreeContextBuffer(pv_context_buffer: *mut c_void) -> i32;
        fn DeleteSecurityContext(ph_context: *mut SecHandle) -> i32;
        fn FreeCredentialsHandle(ph_credential: *mut SecHandle) -> i32;
    }

    /// Frees the Schannel context and credential handle on drop, including on error.
    struct TlsGuards {
        cred: SecHandle,
        has_cred: bool,
        ctxt: SecHandle,
        has_ctxt: bool,
    }

    impl Drop for TlsGuards {
        fn drop(&mut self) {
            unsafe {
                if self.has_ctxt {
                    DeleteSecurityContext(&mut self.ctxt);
                }
                if self.has_cred {
                    FreeCredentialsHandle(&mut self.cred);
                }
            }
        }
    }

    /// Handshake with `host`, write `request`, and return the raw HTTP bytes.
    ///
    /// `max_response` caps the decrypted plaintext. Exceeding it returns
    /// [`NetError::BodyTooLarge`] and does not return a partial body.
    /// The socket timeouts are whatever the caller set. A timeout or `WouldBlock`
    /// with no plaintext yet is an I/O error. The same condition after some
    /// plaintext stops the read and returns what was decrypted.
    pub fn https_exchange(
        mut stream: TcpStream,
        host: &str,
        request: &[u8],
        max_response: usize,
    ) -> Result<Vec<u8>, NetError> {
        let provider = wide("Microsoft Unified Security Protocol Provider");
        let target = wide(host);
        let cred_data = SchannelCred {
            dw_version: SCHANNEL_CRED_VERSION,
            c_creds: 0,
            pa_cred: null(),
            h_root_store: null_mut(),
            c_mappers: 0,
            aph_mappers: null(),
            c_supported_algs: 0,
            palg_supported_algs: null(),
            grbit_enabled_protocols: 0,
            dw_minimum_cipher_strength: 0,
            dw_maximum_cipher_strength: 0,
            dw_session_lifespan: 0,
            // Verification stays on. No client certificate. Strong crypto required.
            dw_flags: SCH_CRED_NO_DEFAULT_CREDS
                | SCH_CRED_AUTO_CRED_VALIDATION
                | SCH_USE_STRONG_CRYPTO,
            dw_cred_format: 0,
        };

        let mut guards = TlsGuards {
            cred: SecHandle::default(),
            has_cred: false,
            ctxt: SecHandle::default(),
            has_ctxt: false,
        };
        let mut expiry = SecurityInteger::default();

        let status = unsafe {
            AcquireCredentialsHandleW(
                null(),
                provider.as_ptr(),
                SECPKG_CRED_OUTBOUND,
                null(),
                (&raw const cred_data).cast::<c_void>(),
                null(),
                null(),
                &mut guards.cred,
                &mut expiry,
            )
        };
        if status != SEC_E_OK {
            return Err(tls_err("could not acquire TLS credentials", status));
        }
        guards.has_cred = true;

        let mut in_buf = Vec::<u8>::new();
        // Replay and sequence detection, confidentiality, and a stream handshake.
        // Supplied creds means no user prompt for a client certificate.
        let flags = ISC_REQ_SEQUENCE_DETECT
            | ISC_REQ_REPLAY_DETECT
            | ISC_REQ_CONFIDENTIALITY
            | ISC_REQ_EXTENDED_ERROR
            | ISC_REQ_ALLOCATE_MEMORY
            | ISC_REQ_STREAM
            | ISC_REQ_USE_SUPPLIED_CREDS;

        loop {
            let mut out_sec = SecBuffer {
                cb_buffer: 0,
                buffer_type: SECBUFFER_TOKEN,
                pv_buffer: null_mut(),
            };
            let mut out_desc = SecBufferDesc {
                ul_version: SECBUFFER_VERSION,
                c_buffers: 1,
                p_buffers: &mut out_sec,
            };
            let mut in_sec = [
                SecBuffer {
                    cb_buffer: in_buf.len() as u32,
                    buffer_type: SECBUFFER_TOKEN,
                    pv_buffer: in_buf.as_mut_ptr().cast::<c_void>(),
                },
                SecBuffer {
                    cb_buffer: 0,
                    buffer_type: SECBUFFER_EMPTY,
                    pv_buffer: null_mut(),
                },
            ];
            let mut in_desc = SecBufferDesc {
                ul_version: SECBUFFER_VERSION,
                c_buffers: 2,
                p_buffers: in_sec.as_mut_ptr(),
            };
            let mut attrs = 0_u32;

            let status = unsafe {
                InitializeSecurityContextW(
                    &mut guards.cred,
                    if guards.has_ctxt {
                        &mut guards.ctxt
                    } else {
                        null_mut()
                    },
                    target.as_ptr(),
                    flags,
                    0,
                    SECURITY_NATIVE_DREP,
                    if guards.has_ctxt {
                        &mut in_desc
                    } else {
                        null_mut()
                    },
                    0,
                    &mut guards.ctxt,
                    &mut out_desc,
                    &mut attrs,
                    &mut expiry,
                )
            };
            if guards.ctxt.dw_lower != 0 || guards.ctxt.dw_upper != 0 {
                guards.has_ctxt = true;
            }

            if !out_sec.pv_buffer.is_null() && out_sec.cb_buffer > 0 {
                let slice = unsafe {
                    std::slice::from_raw_parts(
                        out_sec.pv_buffer.cast::<u8>(),
                        out_sec.cb_buffer as usize,
                    )
                };
                let write_res = stream.write_all(slice);
                unsafe {
                    FreeContextBuffer(out_sec.pv_buffer);
                }
                write_res.map_err(NetError::Io)?;
            } else if !out_sec.pv_buffer.is_null() {
                unsafe {
                    FreeContextBuffer(out_sec.pv_buffer);
                }
            }

            if status == SEC_E_INCOMPLETE_MESSAGE {
                read_more(&mut stream, &mut in_buf)?;
                continue;
            }

            if guards.has_ctxt {
                if in_sec[1].buffer_type == SECBUFFER_EXTRA && in_sec[1].cb_buffer > 0 {
                    let extra = in_sec[1].cb_buffer as usize;
                    let start = in_buf.len().saturating_sub(extra);
                    in_buf.drain(..start);
                } else {
                    in_buf.clear();
                }
            }

            // SEC_E_OK ends the handshake. Schannel has already applied auto validation.
            // Any other status except SEC_I_CONTINUE_NEEDED fails the handshake.
            if status == SEC_E_OK {
                break;
            }
            if status == SEC_I_CONTINUE_NEEDED {
                if in_buf.is_empty() {
                    read_more(&mut stream, &mut in_buf)?;
                }
                continue;
            }
            return Err(tls_err("TLS handshake failed", status));
        }

        let mut sizes = SecPkgContextStreamSizes::default();
        let status = unsafe {
            QueryContextAttributesW(
                &mut guards.ctxt,
                SECPKG_ATTR_STREAM_SIZES,
                (&raw mut sizes).cast::<c_void>(),
            )
        };
        if status != SEC_E_OK {
            return Err(tls_err("could not query TLS stream sizes", status));
        }

        let max_msg = (sizes.cb_maximum_message as usize).max(1024);
        for chunk in request.chunks(max_msg) {
            let header_len = sizes.cb_header as usize;
            let trailer_len = sizes.cb_trailer as usize;
            let mut packet = vec![0_u8; header_len + chunk.len() + trailer_len];
            packet[header_len..header_len + chunk.len()].copy_from_slice(chunk);

            let mut bufs = [
                SecBuffer {
                    cb_buffer: sizes.cb_header,
                    buffer_type: SECBUFFER_STREAM_HEADER,
                    pv_buffer: packet.as_mut_ptr().cast::<c_void>(),
                },
                SecBuffer {
                    cb_buffer: chunk.len() as u32,
                    buffer_type: SECBUFFER_DATA,
                    pv_buffer: unsafe { packet.as_mut_ptr().add(header_len).cast::<c_void>() },
                },
                SecBuffer {
                    cb_buffer: sizes.cb_trailer,
                    buffer_type: SECBUFFER_STREAM_TRAILER,
                    pv_buffer: unsafe {
                        packet
                            .as_mut_ptr()
                            .add(header_len + chunk.len())
                            .cast::<c_void>()
                    },
                },
                SecBuffer {
                    cb_buffer: 0,
                    buffer_type: SECBUFFER_EMPTY,
                    pv_buffer: null_mut(),
                },
            ];
            let mut desc = SecBufferDesc {
                ul_version: SECBUFFER_VERSION,
                c_buffers: 4,
                p_buffers: bufs.as_mut_ptr(),
            };
            let status = unsafe { EncryptMessage(&mut guards.ctxt, 0, &mut desc, 0) };
            if status != SEC_E_OK {
                return Err(tls_err("TLS encryption failed", status));
            }
            let total = (bufs[0].cb_buffer + bufs[1].cb_buffer + bufs[2].cb_buffer) as usize;
            stream.write_all(&packet[..total]).map_err(NetError::Io)?;
        }

        let mut plain = Vec::new();
        let mut temp = [0_u8; 8192];
        loop {
            if !in_buf.is_empty() {
                let mut bufs = [
                    SecBuffer {
                        cb_buffer: in_buf.len() as u32,
                        buffer_type: SECBUFFER_DATA,
                        pv_buffer: in_buf.as_mut_ptr().cast::<c_void>(),
                    },
                    SecBuffer {
                        cb_buffer: 0,
                        buffer_type: SECBUFFER_EMPTY,
                        pv_buffer: null_mut(),
                    },
                    SecBuffer {
                        cb_buffer: 0,
                        buffer_type: SECBUFFER_EMPTY,
                        pv_buffer: null_mut(),
                    },
                    SecBuffer {
                        cb_buffer: 0,
                        buffer_type: SECBUFFER_EMPTY,
                        pv_buffer: null_mut(),
                    },
                ];
                let mut desc = SecBufferDesc {
                    ul_version: SECBUFFER_VERSION,
                    c_buffers: 4,
                    p_buffers: bufs.as_mut_ptr(),
                };
                let status = unsafe { DecryptMessage(&mut guards.ctxt, &mut desc, 0, null_mut()) };
                if status == SEC_E_INCOMPLETE_MESSAGE {
                    match stream.read(&mut temp) {
                        Ok(0) => break,
                        Ok(n) => {
                            in_buf.extend_from_slice(&temp[..n]);
                            continue;
                        }
                        Err(err)
                            if err.kind() == ErrorKind::TimedOut
                                || err.kind() == ErrorKind::WouldBlock =>
                        {
                            if plain.is_empty() {
                                return Err(NetError::Io(err));
                            }
                            break;
                        }
                        Err(err) => return Err(NetError::Io(err)),
                    }
                }
                if status == SEC_I_CONTEXT_EXPIRED {
                    break;
                }
                if status != SEC_E_OK && status != SEC_I_RENEGOTIATE {
                    return Err(tls_err("TLS decryption failed", status));
                }

                let mut extra_slice: Option<Vec<u8>> = None;
                for b in &bufs {
                    if b.buffer_type == SECBUFFER_DATA && b.cb_buffer > 0 && !b.pv_buffer.is_null()
                    {
                        let slice = unsafe {
                            std::slice::from_raw_parts(
                                b.pv_buffer.cast::<u8>(),
                                b.cb_buffer as usize,
                            )
                        };
                        // Cap the decrypted bytes. This is the same limit as the cleartext read.
                        if plain.len() + slice.len() > max_response {
                            return Err(NetError::BodyTooLarge);
                        }
                        plain.extend_from_slice(slice);
                    } else if b.buffer_type == SECBUFFER_EXTRA
                        && b.cb_buffer > 0
                        && !b.pv_buffer.is_null()
                    {
                        let slice = unsafe {
                            std::slice::from_raw_parts(
                                b.pv_buffer.cast::<u8>(),
                                b.cb_buffer as usize,
                            )
                        };
                        extra_slice = Some(slice.to_vec());
                    }
                }
                in_buf = extra_slice.unwrap_or_default();
                // Renegotiation is not completed. The bytes already decrypted are returned.
                if status == SEC_I_RENEGOTIATE {
                    break;
                }
                continue;
            }

            match stream.read(&mut temp) {
                Ok(0) => break,
                Ok(n) => in_buf.extend_from_slice(&temp[..n]),
                Err(err)
                    if err.kind() == ErrorKind::TimedOut || err.kind() == ErrorKind::WouldBlock =>
                {
                    if plain.is_empty() {
                        return Err(NetError::Io(err));
                    }
                    break;
                }
                Err(err) => return Err(NetError::Io(err)),
            }
        }

        Ok(plain)
    }

    /// Read one socket chunk into `buf` during the handshake.
    /// A zero read is an unexpected close, not a clean end.
    fn read_more(stream: &mut TcpStream, buf: &mut Vec<u8>) -> Result<(), NetError> {
        let mut chunk = [0_u8; 4096];
        let n = stream.read(&mut chunk).map_err(NetError::Io)?;
        if n == 0 {
            return Err(NetError::Io(Error::new(
                ErrorKind::UnexpectedEof,
                "the peer closed the connection during TLS handshake",
            )));
        }
        buf.extend_from_slice(&chunk[..n]);
        Ok(())
    }

    /// Wrap a Schannel status as [`NetError::Io`]. `code` is printed as eight hex digits.
    fn tls_err(context: &str, code: i32) -> NetError {
        NetError::Io(Error::other(format!("{context} (0x{:08x})", code as u32)))
    }

    /// UTF-16 string with a trailing NUL for the wide Schannel calls.
    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }
}

#[cfg(target_os = "linux")]
mod imp {
    //! OpenSSL client for one socket.
    //!
    //! Certificate verification is on. `SSL_VERIFY_PEER` is set, the default CA
    //! paths are loaded, and `SSL_get_verify_result` must be `X509_V_OK` before
    //! the HTTP request is written. The plaintext is capped by `max_response`.

    use std::ffi::{CString, c_char, c_int, c_long, c_uint, c_void};
    use std::io::Error;
    use std::net::TcpStream;
    use std::os::fd::AsRawFd;
    use std::ptr::null_mut;

    use crate::NetError;

    // Peer certificate verification is on. There is no verify callback that accepts failures.
    const SSL_VERIFY_PEER: c_int = 0x01;
    const SSL_CTRL_SET_TLSEXT_HOSTNAME: c_int = 55;
    const SSL_CTRL_SET_MIN_PROTO_VERSION: c_int = 123;
    const TLSEXT_NAMETYPE_HOST_NAME: c_long = 0;
    // TLS 1.2 is the minimum. Newer versions remain available through the client method.
    const TLS1_2_VERSION: c_long = 0x0303;
    const X509_V_OK: c_long = 0;

    // OpenSSL symbols. build.rs only adds the library search path.
    // These declarations are what rustc links as ssl and crypto.
    // TLS_client_method through SSL_connect perform the handshake.
    // SSL_write and SSL_read move the HTTP bytes. SSL_shutdown runs on drop.
    #[link(name = "ssl")]
    #[link(name = "crypto")]
    unsafe extern "C" {
        fn TLS_client_method() -> *const c_void;
        fn SSL_CTX_new(method: *const c_void) -> *mut c_void;
        fn SSL_CTX_free(ctx: *mut c_void);
        fn SSL_CTX_set_default_verify_paths(ctx: *mut c_void) -> c_int;
        fn SSL_CTX_set_verify(ctx: *mut c_void, mode: c_int, callback: *const c_void);
        fn SSL_CTX_ctrl(ctx: *mut c_void, cmd: c_int, larg: c_long, parg: *mut c_void) -> c_long;
        fn SSL_new(ctx: *mut c_void) -> *mut c_void;
        fn SSL_free(ssl: *mut c_void);
        fn SSL_set_fd(ssl: *mut c_void, fd: c_int) -> c_int;
        fn SSL_ctrl(ssl: *mut c_void, cmd: c_int, larg: c_long, parg: *mut c_void) -> c_long;
        fn SSL_get0_param(ssl: *mut c_void) -> *mut c_void;
        fn X509_VERIFY_PARAM_set1_host(
            param: *mut c_void,
            name: *const c_char,
            namelen: usize,
        ) -> c_int;
        fn SSL_connect(ssl: *mut c_void) -> c_int;
        fn SSL_get_verify_result(ssl: *const c_void) -> c_long;
        fn SSL_write(ssl: *mut c_void, buf: *const c_void, num: c_int) -> c_int;
        fn SSL_read(ssl: *mut c_void, buf: *mut c_void, num: c_int) -> c_int;
        fn SSL_shutdown(ssl: *mut c_void) -> c_int;
        fn SSL_set_alpn_protos(ssl: *mut c_void, protos: *const u8, protos_len: c_uint) -> c_int;
        fn SSL_get0_alpn_selected(ssl: *const c_void, data: *mut *const u8, len: *mut c_uint);
    }

    /// Shuts down and frees the SSL session, then frees the context, on drop.
    struct OpenSslGuard {
        ctx: *mut c_void,
        ssl: *mut c_void,
    }

    impl Drop for OpenSslGuard {
        fn drop(&mut self) {
            unsafe {
                if !self.ssl.is_null() {
                    let _ = SSL_shutdown(self.ssl);
                    SSL_free(self.ssl);
                }
                if !self.ctx.is_null() {
                    SSL_CTX_free(self.ctx);
                }
            }
        }
    }

    /// One verified TLS connection. Owns the socket and the OpenSSL session.
    ///
    /// HTTP/2 uses this directly because it reads and writes in turns. The
    /// HTTP/1.1 path goes through [`https_exchange`].
    pub struct TlsSession {
        guard: OpenSslGuard,
        // Kept open for as long as OpenSSL uses its descriptor.
        _stream: TcpStream,
    }

    impl TlsSession {
        /// Handshake with `host` over `stream`, offering `alpn` protocols in
        /// order of preference (for example `h2` then `http/1.1`). An empty
        /// list sends no ALPN extension.
        ///
        /// Verification is on. The system CA paths must load, `SSL_connect` must
        /// succeed, and `SSL_get_verify_result` must be `X509_V_OK`. A failed
        /// certificate check returns an I/O error. The SNI control return value
        /// is ignored. If the hostname check cannot be set, the connection fails
        /// before the handshake.
        pub fn connect(stream: TcpStream, host: &str, alpn: &[&[u8]]) -> Result<Self, NetError> {
            let host_c = CString::new(host)
                .map_err(|_| NetError::Io(Error::other("invalid TLS hostname")))?;
            let alpn_wire = alpn_wire_format(alpn)?;

            let mut guard = OpenSslGuard {
                ctx: null_mut(),
                ssl: null_mut(),
            };

            unsafe {
                let method = TLS_client_method();
                if method.is_null() {
                    return Err(NetError::Io(Error::other(
                        "could not initialize OpenSSL TLS client method",
                    )));
                }
                guard.ctx = SSL_CTX_new(method);
                if guard.ctx.is_null() {
                    return Err(NetError::Io(Error::other(
                        "could not allocate OpenSSL context",
                    )));
                }
                if SSL_CTX_set_default_verify_paths(guard.ctx) != 1 {
                    return Err(NetError::Io(Error::other(
                        "could not load system CA certificate paths",
                    )));
                }
                // Verification is on. The null callback keeps OpenSSL's default check.
                SSL_CTX_set_verify(guard.ctx, SSL_VERIFY_PEER, null_mut());
                let _ = SSL_CTX_ctrl(
                    guard.ctx,
                    SSL_CTRL_SET_MIN_PROTO_VERSION,
                    TLS1_2_VERSION,
                    null_mut(),
                );

                guard.ssl = SSL_new(guard.ctx);
                if guard.ssl.is_null() {
                    return Err(NetError::Io(Error::other(
                        "could not allocate OpenSSL session",
                    )));
                }

                let _ = SSL_ctrl(
                    guard.ssl,
                    SSL_CTRL_SET_TLSEXT_HOSTNAME,
                    TLSEXT_NAMETYPE_HOST_NAME,
                    host_c.as_ptr().cast_mut().cast::<c_void>(),
                );

                // Hostname check. Without it any trusted certificate would pass,
                // whatever name it was issued for, so a failed set ends the connection.
                let param = SSL_get0_param(guard.ssl);
                if param.is_null()
                    || X509_VERIFY_PARAM_set1_host(param, host_c.as_ptr(), host.len()) != 1
                {
                    return Err(NetError::Io(Error::other(
                        "could not set the TLS hostname check",
                    )));
                }

                // ALPN list. OpenSSL copies it; this call returns 0 on success.
                if !alpn_wire.is_empty()
                    && SSL_set_alpn_protos(guard.ssl, alpn_wire.as_ptr(), alpn_wire.len() as c_uint)
                        != 0
                {
                    return Err(NetError::Io(Error::other(
                        "could not set the TLS protocol list",
                    )));
                }

                if SSL_set_fd(guard.ssl, stream.as_raw_fd()) != 1 {
                    return Err(NetError::Io(Error::other(
                        "could not bind socket descriptor to OpenSSL session",
                    )));
                }

                if SSL_connect(guard.ssl) != 1 {
                    return Err(NetError::Io(Error::other("OpenSSL TLS handshake failed")));
                }

                // Certificate verification is on. A non-OK result aborts before any HTTP write.
                if SSL_get_verify_result(guard.ssl) != X509_V_OK {
                    return Err(NetError::Io(Error::other(
                        "TLS peer certificate verification failed",
                    )));
                }
            }
            Ok(Self {
                guard,
                _stream: stream,
            })
        }

        /// The protocol the server picked through ALPN, if it picked one.
        pub fn alpn_protocol(&self) -> Option<Vec<u8>> {
            let mut data: *const u8 = std::ptr::null();
            let mut len: c_uint = 0;
            // OpenSSL points `data` into the session; the bytes are copied out
            // before the session can go away.
            unsafe {
                SSL_get0_alpn_selected(self.guard.ssl, &mut data, &mut len);
                if data.is_null() || len == 0 {
                    return None;
                }
                Some(std::slice::from_raw_parts(data, len as usize).to_vec())
            }
        }

        /// Write all of `data`.
        pub fn write_all(&mut self, data: &[u8]) -> Result<(), NetError> {
            let mut offset = 0usize;
            while offset < data.len() {
                let chunk_len = (data.len() - offset).min(16384) as c_int;
                let written = unsafe {
                    SSL_write(
                        self.guard.ssl,
                        data[offset..].as_ptr().cast::<c_void>(),
                        chunk_len,
                    )
                };
                if written <= 0 {
                    return Err(NetError::Io(Error::other("OpenSSL TLS write failed")));
                }
                offset += written as usize;
            }
            Ok(())
        }

        /// Read some plaintext into `buf`. Zero means the connection ended,
        /// timed out, or failed.
        pub fn read(&mut self, buf: &mut [u8]) -> usize {
            let want = buf.len().min(c_int::MAX as usize) as c_int;
            let n = unsafe { SSL_read(self.guard.ssl, buf.as_mut_ptr().cast::<c_void>(), want) };
            if n <= 0 { 0 } else { n as usize }
        }
    }

    /// ALPN protocol names in the TLS wire format: each name with a one-byte length.
    fn alpn_wire_format(protocols: &[&[u8]]) -> Result<Vec<u8>, NetError> {
        let mut wire = Vec::new();
        for name in protocols {
            if name.is_empty() || name.len() > 255 {
                return Err(NetError::Io(Error::other("invalid TLS protocol name")));
            }
            wire.push(name.len() as u8);
            wire.extend_from_slice(name);
        }
        Ok(wire)
    }

    /// Handshake with `host`, write `request`, and return the raw HTTP bytes.
    ///
    /// No ALPN list is sent, so the server speaks HTTP/1.1. `max_response`
    /// caps the plaintext. A short read (`SSL_read` <= 0) ends the response,
    /// including a clean close.
    pub fn https_exchange(
        stream: TcpStream,
        host: &str,
        request: &[u8],
        max_response: usize,
    ) -> Result<Vec<u8>, NetError> {
        let mut session = TlsSession::connect(stream, host, &[])?;
        session.write_all(request)?;
        read_to_end(&mut session, max_response)
    }

    /// Read until the peer closes. More than `max_response` bytes is an error.
    pub fn read_to_end(session: &mut TlsSession, max_response: usize) -> Result<Vec<u8>, NetError> {
        let mut plain = Vec::new();
        let mut buf = [0u8; 8192];
        loop {
            let count = session.read(&mut buf);
            if count == 0 {
                break;
            }
            // Cap the decrypted bytes. A larger response is not returned in part.
            if plain.len() + count > max_response {
                return Err(NetError::BodyTooLarge);
            }
            plain.extend_from_slice(&buf[..count]);
        }
        Ok(plain)
    }
}

/// TLS exchange for the current target. Windows or Linux only.
pub use imp::https_exchange;
/// A TLS connection with ALPN, for HTTP/2. Linux only for now; Windows
/// requests stay on HTTP/1.1.
#[cfg(target_os = "linux")]
pub use imp::{TlsSession, read_to_end};

/// Local handshakes against `openssl s_server` with certificates made for the
/// run. They need the `openssl` command line tool; without it they print a
/// note and return.
#[cfg(all(test, target_os = "linux"))]
mod tests {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant};

    const CA_FILE_VAR: &str = "SSL_CERT_FILE";
    const PORT_VAR: &str = "ANVIL_NET_TLS_TEST_PORT";
    const REQUEST: &[u8] = b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";

    struct Server {
        child: Child,
        dir: PathBuf,
        port: u16,
    }

    impl Drop for Server {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn openssl(dir: &Path, args: &[&str]) -> bool {
        Command::new("openssl")
            .args(args)
            .current_dir(dir)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }

    /// Make a throwaway CA and a `localhost` certificate it signed, then serve
    /// that certificate on a free local port. `None` when `openssl` is missing.
    fn start_server(tag: &str) -> Option<Server> {
        if !openssl(&std::env::temp_dir(), &["version"]) {
            eprintln!("openssl command not found; skipping the local TLS test");
            return None;
        }
        let dir = std::env::temp_dir().join(format!("anvil-net-tls-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(openssl(
            &dir,
            &[
                "req",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-days",
                "1",
                "-subj",
                "/CN=Anvil test CA",
                "-keyout",
                "ca.key",
                "-out",
                "ca.pem",
            ],
        ));
        assert!(openssl(
            &dir,
            &[
                "req",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-days",
                "1",
                "-subj",
                "/CN=localhost",
                "-addext",
                "subjectAltName=DNS:localhost",
                "-CA",
                "ca.pem",
                "-CAkey",
                "ca.key",
                "-keyout",
                "leaf.key",
                "-out",
                "leaf.pem",
            ],
        ));
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let child = Command::new("openssl")
            .args([
                "s_server", "-quiet", "-www", "-cert", "leaf.pem", "-key", "leaf.key",
            ])
            .args(["-accept", &port.to_string()])
            .current_dir(&dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let server = Server { child, dir, port };
        let deadline = Instant::now() + Duration::from_secs(10);
        while TcpStream::connect(("127.0.0.1", port)).is_err() {
            assert!(Instant::now() < deadline, "test TLS server did not start");
            std::thread::sleep(Duration::from_millis(50));
        }
        Some(server)
    }

    fn exchange(port: u16, host: &str) -> Result<Vec<u8>, crate::NetError> {
        let stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        super::https_exchange(stream, host, REQUEST, 1 << 20)
    }

    #[test]
    fn refuses_a_certificate_from_an_unknown_ca() {
        let Some(server) = start_server("untrusted") else {
            return;
        };
        // The test CA is not in the system store, so this must fail.
        let err = exchange(server.port, "localhost").expect_err("untrusted certificate accepted");
        assert!(matches!(err, crate::NetError::Io(_)), "{err:?}");
    }

    #[test]
    fn checks_the_host_name_against_a_trusted_certificate() {
        let Some(server) = start_server("hostname") else {
            return;
        };
        // Run the checks in a child test process that trusts the test CA
        // through its environment, so this process keeps the system store.
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "tls::tests::trusted_ca_child",
                "--ignored",
                "--nocapture",
            ])
            .env(CA_FILE_VAR, server.dir.join("ca.pem"))
            .env(PORT_VAR, server.port.to_string())
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&output.stdout);
        assert!(output.status.success(), "{text}");
        assert!(text.contains("1 passed"), "{text}");
    }

    /// Started by `checks_the_host_name_against_a_trusted_certificate`.
    #[test]
    #[ignore = "run through checks_the_host_name_against_a_trusted_certificate"]
    fn trusted_ca_child() {
        let port: u16 = std::env::var(PORT_VAR)
            .expect("port from the parent test")
            .parse()
            .unwrap();
        let mut plain = Vec::new();
        exchange(port, "localhost")
            .expect("trusted certificate for localhost")
            .as_slice()
            .read_to_end(&mut plain)
            .unwrap();
        assert!(
            plain.starts_with(b"HTTP/1."),
            "{:?}",
            String::from_utf8_lossy(&plain)
        );

        let err = exchange(port, "not-localhost.test")
            .expect_err("certificate for another name accepted");
        assert!(matches!(err, crate::NetError::Io(_)), "{err:?}");
        let _ = std::io::stdout().flush();
    }
}
