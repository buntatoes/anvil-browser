//! TLS client stream over `TcpStream` using Windows Schannel (`secur32.dll`).

use std::ffi::c_void;
use std::io::{Error, ErrorKind, Read, Write};
use std::net::TcpStream;
use std::ptr::{null, null_mut};

use crate::NetError;

const SECPKG_CRED_OUTBOUND: u32 = 0x0000_0002;
const SCHANNEL_CRED_VERSION: u32 = 4;
const SCH_CRED_NO_DEFAULT_CREDS: u32 = 0x0000_0010;
const SCH_CRED_AUTO_CRED_VALIDATION: u32 = 0x0000_0020;
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
        dw_flags: SCH_CRED_NO_DEFAULT_CREDS | SCH_CRED_AUTO_CRED_VALIDATION | SCH_USE_STRONG_CRYPTO,
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
                if b.buffer_type == SECBUFFER_DATA && b.cb_buffer > 0 && !b.pv_buffer.is_null() {
                    let slice = unsafe {
                        std::slice::from_raw_parts(b.pv_buffer.cast::<u8>(), b.cb_buffer as usize)
                    };
                    if plain.len() + slice.len() > max_response {
                        return Err(NetError::BodyTooLarge);
                    }
                    plain.extend_from_slice(slice);
                } else if b.buffer_type == SECBUFFER_EXTRA
                    && b.cb_buffer > 0
                    && !b.pv_buffer.is_null()
                {
                    let slice = unsafe {
                        std::slice::from_raw_parts(b.pv_buffer.cast::<u8>(), b.cb_buffer as usize)
                    };
                    extra_slice = Some(slice.to_vec());
                }
            }
            in_buf = extra_slice.unwrap_or_default();
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

fn tls_err(context: &str, code: i32) -> NetError {
    NetError::Io(Error::other(format!("{context} (0x{:08x})", code as u32)))
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}
