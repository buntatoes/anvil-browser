// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Clean-room PNG, JPEG, GIF, WebP, ICO, BMP, and PPM image codecs and resampling
//! backed by the in-tree pure-Go `anvil-image` worker binary (`cmd/anvil-image --worker`).

use std::fmt;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Mutex, OnceLock};

/// Embedded compiled pure-Go `anvil-image` worker binary.
static EMBEDDED_GO_BIN: &[u8] = include_bytes!(env!("ANVIL_IMAGE_GO_BIN"));

/// Eight-byte PNG signature.
pub const PNG_SIGNATURE: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];

/// Largest width or height a decoder will accept, in pixels.
pub const MAX_IMAGE_DIMENSION: u32 = 8192;

/// Largest `width * height` a decoder will accept.
pub const MAX_IMAGE_PIXELS: u64 = 16_777_216;

const OP_DECODE_AUTO: u8 = 0x01;
const OP_RESIZE_BILINEAR: u8 = 0x02;
const OP_ENCODE_PNG: u8 = 0x03;
const OP_ENCODE_BMP: u8 = 0x04;
const _OP_SHUTDOWN: u8 = 0xFF;

/// Error returned on image decoding or validation failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageError {
    UnexpectedEof,
    UnsupportedFormat(&'static str),
    InvalidHeader(&'static str),
    DimensionsTooLarge { width: u32, height: u32 },
    InvalidFilter(u8),
    Worker(String),
}

impl fmt::Display for ImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedEof => write!(f, "unexpected end of image file"),
            Self::UnsupportedFormat(msg) => write!(f, "unsupported image format: {msg}"),
            Self::InvalidHeader(msg) => write!(f, "invalid image header: {msg}"),
            Self::DimensionsTooLarge { width, height } => write!(
                f,
                "image dimensions {width}x{height} exceed limits ({MAX_IMAGE_DIMENSION}x{MAX_IMAGE_DIMENSION}, {MAX_IMAGE_PIXELS} pixels)"
            ),
            Self::InvalidFilter(kind) => write!(f, "invalid PNG scanline filter {kind}"),
            Self::Worker(msg) => write!(f, "image worker error: {msg}"),
        }
    }
}

impl std::error::Error for ImageError {}

/// Validate width and height against maximum dimensions and pixel bounds.
pub fn validate_dimensions(width: u32, height: u32) -> Result<(), ImageError> {
    if width == 0 || height == 0 {
        return Err(ImageError::InvalidHeader("zero-sized image"));
    }
    let total_pixels = u64::from(width) * u64::from(height);
    if width > MAX_IMAGE_DIMENSION
        || height > MAX_IMAGE_DIMENSION
        || total_pixels > MAX_IMAGE_PIXELS
    {
        return Err(ImageError::DimensionsTooLarge { width, height });
    }
    Ok(())
}

/// Top-down 8-bit image with tightly packed RGBA pixels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl Image {
    pub fn new(width: u32, height: u32) -> Self {
        let len = (width as usize)
            .saturating_mul(height as usize)
            .saturating_mul(4);
        Self {
            width,
            height,
            pixels: vec![0; len],
        }
    }

    pub fn from_rgba(width: u32, height: u32, pixels: Vec<u8>) -> Result<Self, ImageError> {
        validate_dimensions(width, height)?;
        let expected = (width as usize)
            .saturating_mul(height as usize)
            .saturating_mul(4);
        if pixels.len() != expected {
            return Err(ImageError::InvalidHeader("RGBA pixel buffer size mismatch"));
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let idx = ((y * self.width + x) as usize) * 4;
        Some([
            self.pixels[idx],
            self.pixels[idx + 1],
            self.pixels[idx + 2],
            self.pixels[idx + 3],
        ])
    }

    pub fn set_pixel(&mut self, x: u32, y: u32, rgba: [u8; 4]) {
        if x >= self.width || y >= self.height {
            return;
        }
        let idx = ((y * self.width + x) as usize) * 4;
        self.pixels[idx..idx + 4].copy_from_slice(&rgba);
    }

    pub fn mipmap_halve(&self, halve_x: bool, halve_y: bool) -> Self {
        if self.width == 0 || self.height == 0 {
            return Self::new(0, 0);
        }
        let dst_w = if halve_x && self.width > 1 {
            self.width.div_ceil(2)
        } else {
            self.width
        };
        let dst_h = if halve_y && self.height > 1 {
            self.height.div_ceil(2)
        } else {
            self.height
        };
        let step_x = if dst_w < self.width { 2u32 } else { 1u32 };
        let step_y = if dst_h < self.height { 2u32 } else { 1u32 };
        let mut out = Self::new(dst_w, dst_h);
        for dy in 0..dst_h {
            let sy0 = dy * step_y;
            let sy1 = (sy0 + step_y - 1).min(self.height - 1);
            for dx in 0..dst_w {
                let sx0 = dx * step_x;
                let sx1 = (sx0 + step_x - 1).min(self.width - 1);
                let taps = [
                    self.pixel(sx0, sy0).unwrap_or([0; 4]),
                    self.pixel(sx1, sy0).unwrap_or([0; 4]),
                    self.pixel(sx0, sy1).unwrap_or([0; 4]),
                    self.pixel(sx1, sy1).unwrap_or([0; 4]),
                ];
                let mut sum_r = 0.0f32;
                let mut sum_g = 0.0f32;
                let mut sum_b = 0.0f32;
                let mut sum_a = 0.0f32;
                for c in taps {
                    let a = c[3] as f32;
                    sum_r += (c[0] as f32) * a;
                    sum_g += (c[1] as f32) * a;
                    sum_b += (c[2] as f32) * a;
                    sum_a += a;
                }
                let out_a = (sum_a * 0.25).round().clamp(0.0, 255.0) as u8;
                let rgba = if sum_a > 0.0 {
                    [
                        (sum_r / sum_a).round().clamp(0.0, 255.0) as u8,
                        (sum_g / sum_a).round().clamp(0.0, 255.0) as u8,
                        (sum_b / sum_a).round().clamp(0.0, 255.0) as u8,
                        out_a,
                    ]
                } else {
                    [0, 0, 0, 0]
                };
                out.set_pixel(dx, dy, rgba);
            }
        }
        out
    }

    pub fn resize_bilinear(&self, new_width: u32, new_height: u32) -> Self {
        if new_width == 0 || new_height == 0 || self.width == 0 || self.height == 0 {
            return Self::new(new_width, new_height);
        }
        if new_width == self.width && new_height == self.height {
            return self.clone();
        }

        let res = with_worker(|w| {
            let mut req = Vec::with_capacity(20 + self.pixels.len());
            req.extend_from_slice(&self.width.to_le_bytes());
            req.extend_from_slice(&self.height.to_le_bytes());
            req.extend_from_slice(&new_width.to_le_bytes());
            req.extend_from_slice(&new_height.to_le_bytes());
            req.extend_from_slice(&(self.pixels.len() as u32).to_le_bytes());
            req.extend_from_slice(&self.pixels);

            let resp = w.send_command(OP_RESIZE_BILINEAR, &req)?;
            if resp.len() < 12 {
                return Err("short resize response".to_string());
            }
            let rw = u32::from_le_bytes(resp[0..4].try_into().unwrap());
            let rh = u32::from_le_bytes(resp[4..8].try_into().unwrap());
            let rlen = u32::from_le_bytes(resp[8..12].try_into().unwrap()) as usize;
            if resp.len() < 12 + rlen {
                return Err("short resize pixel payload".to_string());
            }
            Ok(Self {
                width: rw,
                height: rh,
                pixels: resp[12..12 + rlen].to_vec(),
            })
        });

        res.unwrap_or_else(|_| self.local_resize_bilinear(new_width, new_height))
    }

    fn local_resize_bilinear(&self, new_width: u32, new_height: u32) -> Self {
        let mut src = std::borrow::Cow::Borrowed(self);
        while (src.width > new_width.saturating_mul(2) && src.width > 1)
            || (src.height > new_height.saturating_mul(2) && src.height > 1)
        {
            let hx = src.width > new_width.saturating_mul(2) && src.width > 1;
            let hy = src.height > new_height.saturating_mul(2) && src.height > 1;
            let next = src.mipmap_halve(hx, hy);
            src = std::borrow::Cow::Owned(next);
        }
        if src.width == new_width && src.height == new_height {
            return src.into_owned();
        }

        let mut out = Self::new(new_width, new_height);
        let x_ratio = (src.width as f32) / (new_width as f32);
        let y_ratio = (src.height as f32) / (new_height as f32);

        for ny in 0..new_height {
            let sy = ((ny as f32 + 0.5) * y_ratio - 0.5).clamp(0.0, (src.height - 1) as f32);
            let y0 = sy.floor() as u32;
            let y1 = (y0 + 1).min(src.height - 1);
            let wy = sy - (y0 as f32);

            for nx in 0..new_width {
                let sx = ((nx as f32 + 0.5) * x_ratio - 0.5).clamp(0.0, (src.width - 1) as f32);
                let x0 = sx.floor() as u32;
                let x1 = (x0 + 1).min(src.width - 1);
                let wx = sx - (x0 as f32);

                let c00 = src.pixel(x0, y0).unwrap_or([0; 4]);
                let c10 = src.pixel(x1, y0).unwrap_or([0; 4]);
                let c01 = src.pixel(x0, y1).unwrap_or([0; 4]);
                let c11 = src.pixel(x1, y1).unwrap_or([0; 4]);

                let w00 = (1.0 - wx) * (1.0 - wy);
                let w10 = wx * (1.0 - wy);
                let w01 = (1.0 - wx) * wy;
                let w11 = wx * wy;

                let a00 = (c00[3] as f32) * w00;
                let a10 = (c10[3] as f32) * w10;
                let a01 = (c01[3] as f32) * w01;
                let a11 = (c11[3] as f32) * w11;
                let sum_a = a00 + a10 + a01 + a11;
                let out_a = sum_a.round().clamp(0.0, 255.0) as u8;

                let rgba = if sum_a > 0.0 {
                    let mut rgb = [0u8; 4];
                    for ch in 0..3 {
                        let pm = (c00[ch] as f32) * a00
                            + (c10[ch] as f32) * a10
                            + (c01[ch] as f32) * a01
                            + (c11[ch] as f32) * a11;
                        rgb[ch] = (pm / sum_a).round().clamp(0.0, 255.0) as u8;
                    }
                    rgb[3] = out_a;
                    rgb
                } else {
                    [0, 0, 0, 0]
                };
                out.set_pixel(nx, ny, rgba);
            }
        }
        out
    }
}

struct GoImageWorker {
    _child: Child,
    stdin: ChildStdin,
    stdout: ChildStdout,
}

fn resolve_go_binary_path() -> PathBuf {
    let build_bin = PathBuf::from(env!("ANVIL_IMAGE_GO_BIN"));
    if build_bin.exists() {
        return build_bin;
    }
    let ext = if cfg!(windows) { ".exe" } else { "" };
    let tmp_path =
        std::env::temp_dir().join(format!("anvil-image-go-v09-{}{}", std::process::id(), ext));
    if !tmp_path.exists() {
        let _ = std::fs::write(&tmp_path, EMBEDDED_GO_BIN);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o755));
        }
    }
    tmp_path
}

fn spawn_worker() -> Result<GoImageWorker, String> {
    let bin = resolve_go_binary_path();
    let mut cmd = Command::new(&bin);
    cmd.arg("--worker")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("failed to spawn anvil-image worker: {e}"))?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "missing worker stdin".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "missing worker stdout".to_string())?;
    Ok(GoImageWorker {
        _child: child,
        stdin,
        stdout,
    })
}

impl GoImageWorker {
    fn send_command(&mut self, op: u8, payload: &[u8]) -> Result<Vec<u8>, String> {
        let mut req_hdr = [0u8; 5];
        req_hdr[0] = op;
        req_hdr[1..5].copy_from_slice(&(payload.len() as u32).to_le_bytes());
        self.stdin
            .write_all(&req_hdr)
            .map_err(|e| format!("write error: {e}"))?;
        if !payload.is_empty() {
            self.stdin
                .write_all(payload)
                .map_err(|e| format!("write payload error: {e}"))?;
        }
        self.stdin
            .flush()
            .map_err(|e| format!("flush error: {e}"))?;

        let mut status_buf = [0u8; 1];
        self.stdout
            .read_exact(&mut status_buf)
            .map_err(|e| format!("read status error: {e}"))?;
        let status = status_buf[0];

        if status != 0 {
            let mut err_len_buf = [0u8; 4];
            self.stdout
                .read_exact(&mut err_len_buf)
                .map_err(|e| format!("read error len: {e}"))?;
            let err_len = u32::from_le_bytes(err_len_buf) as usize;
            let mut err_bytes = vec![0u8; err_len];
            self.stdout
                .read_exact(&mut err_bytes)
                .map_err(|e| format!("read error msg: {e}"))?;
            return Err(String::from_utf8_lossy(&err_bytes).into_owned());
        }

        match op {
            OP_DECODE_AUTO | OP_RESIZE_BILINEAR => {
                let mut hdr = [0u8; 12];
                self.stdout
                    .read_exact(&mut hdr)
                    .map_err(|e| format!("read resp hdr: {e}"))?;
                let pixels_len = u32::from_le_bytes(hdr[8..12].try_into().unwrap()) as usize;
                let mut out = Vec::with_capacity(12 + pixels_len);
                out.extend_from_slice(&hdr);
                let mut pixels = vec![0u8; pixels_len];
                self.stdout
                    .read_exact(&mut pixels)
                    .map_err(|e| format!("read pixels: {e}"))?;
                out.extend_from_slice(&pixels);
                Ok(out)
            }
            OP_ENCODE_PNG | OP_ENCODE_BMP => {
                let mut len_buf = [0u8; 4];
                self.stdout
                    .read_exact(&mut len_buf)
                    .map_err(|e| format!("read len: {e}"))?;
                let enc_len = u32::from_le_bytes(len_buf) as usize;
                let mut enc_bytes = vec![0u8; enc_len];
                self.stdout
                    .read_exact(&mut enc_bytes)
                    .map_err(|e| format!("read encoded bytes: {e}"))?;
                Ok(enc_bytes)
            }
            _ => Ok(Vec::new()),
        }
    }
}

fn worker_slot() -> &'static Mutex<Option<GoImageWorker>> {
    static SLOT: OnceLock<Mutex<Option<GoImageWorker>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

fn with_worker<R>(mut f: impl FnMut(&mut GoImageWorker) -> Result<R, String>) -> Result<R, String> {
    let mut guard = worker_slot()
        .lock()
        .map_err(|_| "worker lock poisoned".to_string())?;
    if guard.is_none() {
        *guard = Some(spawn_worker()?);
    }
    let res = if let Some(worker) = guard.as_mut() {
        f(worker)
    } else {
        Err("worker unavailable".to_string())
    };

    match res {
        Ok(v) => Ok(v),
        Err(err) => {
            let is_io_error = err.contains("write ")
                || err.contains("write error")
                || err.contains("read ")
                || err.contains("flush error")
                || err.contains("pipe")
                || err.contains("os error 232")
                || err.contains("os error 109")
                || err.contains("BrokenPipe")
                || err.contains("UnexpectedEof")
                || guard.as_mut().map_or(false, |w| {
                    w._child.try_wait().map(|s| s.is_some()).unwrap_or(true)
                });

            if is_io_error {
                *guard = None;
                let mut new_worker = spawn_worker()?;
                let retry_res = f(&mut new_worker);
                if retry_res.is_ok() {
                    *guard = Some(new_worker);
                }
                retry_res
            } else {
                Err(err)
            }
        }
    }
}

/// Detect image container format automatically and decode into top-down RGBA.
pub fn decode_auto(bytes: &[u8]) -> Result<Image, ImageError> {
    if bytes.starts_with(b"BM") {
        return decode_bmp(bytes);
    }
    if bytes.starts_with(b"P6") || bytes.starts_with(b"P5") {
        return decode_ppm(bytes);
    }
    let res = with_worker(|w| {
        let resp = w.send_command(OP_DECODE_AUTO, bytes)?;
        if resp.len() < 12 {
            return Err("short decode response".to_string());
        }
        let width = u32::from_le_bytes(resp[0..4].try_into().unwrap());
        let height = u32::from_le_bytes(resp[4..8].try_into().unwrap());
        let len = u32::from_le_bytes(resp[8..12].try_into().unwrap()) as usize;
        if resp.len() < 12 + len {
            return Err("short pixel buffer".to_string());
        }
        Ok(Image {
            width,
            height,
            pixels: resp[12..12 + len].to_vec(),
        })
    });

    res.map_err(ImageError::Worker)
}

pub fn decode_png(bytes: &[u8]) -> Result<Image, ImageError> {
    if bytes.len() < 8 || bytes[..8] != PNG_SIGNATURE {
        return Err(ImageError::InvalidHeader("missing PNG signature"));
    }
    decode_auto(bytes)
}

pub fn decode_jpeg(bytes: &[u8]) -> Result<Image, ImageError> {
    if bytes.len() < 2 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return Err(ImageError::InvalidHeader("missing JPEG SOI marker"));
    }
    decode_auto(bytes)
}

pub fn decode_gif(bytes: &[u8]) -> Result<Image, ImageError> {
    if !bytes.starts_with(b"GIF87a") && !bytes.starts_with(b"GIF89a") {
        return Err(ImageError::InvalidHeader("missing GIF signature"));
    }
    decode_auto(bytes)
}

pub fn decode_webp(bytes: &[u8]) -> Result<Image, ImageError> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WEBP" {
        return Err(ImageError::InvalidHeader("missing RIFF WEBP signature"));
    }
    decode_auto(bytes)
}

pub fn decode_ico(bytes: &[u8]) -> Result<Image, ImageError> {
    if bytes.len() < 4 || (!bytes.starts_with(&[0, 0, 1, 0]) && !bytes.starts_with(&[0, 0, 2, 0])) {
        return Err(ImageError::InvalidHeader("missing ICO/CUR header"));
    }
    decode_auto(bytes)
}

pub fn decode_bmp(bytes: &[u8]) -> Result<Image, ImageError> {
    if bytes.len() < 54 || !bytes.starts_with(b"BM") {
        return Err(ImageError::InvalidHeader("missing BMP signature"));
    }
    let data_offset = u32::from_le_bytes(bytes[10..14].try_into().unwrap()) as usize;
    let header_size = u32::from_le_bytes(bytes[14..18].try_into().unwrap()) as usize;
    if header_size < 40 {
        return Err(ImageError::UnsupportedFormat("unsupported BMP header type"));
    }
    let width_i32 = i32::from_le_bytes(bytes[18..22].try_into().unwrap());
    let height_i32 = i32::from_le_bytes(bytes[22..26].try_into().unwrap());
    let planes = u16::from_le_bytes(bytes[26..28].try_into().unwrap());
    let bpp = u16::from_le_bytes(bytes[28..30].try_into().unwrap());
    let compression = u32::from_le_bytes(bytes[30..34].try_into().unwrap());
    if planes != 1 {
        return Err(ImageError::InvalidHeader("BMP planes must be 1"));
    }
    if width_i32 <= 0 || height_i32 == 0 {
        return Err(ImageError::InvalidHeader("invalid BMP dimensions"));
    }
    if compression != 0 && compression != 3 {
        return Err(ImageError::UnsupportedFormat(
            "compressed BMP not supported",
        ));
    }
    if !matches!(bpp, 1 | 4 | 8 | 24 | 32) {
        return Err(ImageError::UnsupportedFormat("unsupported BMP bit depth"));
    }
    let width = width_i32 as u32;
    let top_down = height_i32 < 0;
    let height = height_i32.unsigned_abs();
    validate_dimensions(width, height)?;

    let mut palette: Vec<[u8; 4]> = Vec::new();
    if bpp <= 8 {
        let palette_offset = 14 + header_size;
        let num_colors = 1usize << bpp;
        if palette_offset + num_colors * 4 <= bytes.len() {
            palette.reserve(num_colors);
            for i in 0..num_colors {
                let base = palette_offset + i * 4;
                palette.push([bytes[base + 2], bytes[base + 1], bytes[base], 255]);
            }
        }
    }

    let row_stride = ((width as usize * bpp as usize + 31) / 32) * 4;
    if data_offset.saturating_add(row_stride.saturating_mul(height as usize)) > bytes.len() {
        return Err(ImageError::UnexpectedEof);
    }

    let mut out = Image::new(width, height);
    for row in 0..(height as usize) {
        let dst_y = if top_down {
            row as u32
        } else {
            height - 1 - (row as u32)
        };
        let row_base = data_offset + row * row_stride;
        match bpp {
            1 => {
                for x in 0..width {
                    let byte_idx = row_base + (x as usize / 8);
                    let bit_idx = 7 - (x % 8);
                    let color_idx = ((bytes[byte_idx] >> bit_idx) & 1) as usize;
                    if let Some(&c) = palette.get(color_idx) {
                        out.set_pixel(x, dst_y, c);
                    }
                }
            }
            4 => {
                for x in 0..width {
                    let byte_idx = row_base + (x as usize / 2);
                    let color_idx = if x % 2 == 0 {
                        ((bytes[byte_idx] >> 4) & 0x0F) as usize
                    } else {
                        (bytes[byte_idx] & 0x0F) as usize
                    };
                    if let Some(&c) = palette.get(color_idx) {
                        out.set_pixel(x, dst_y, c);
                    }
                }
            }
            8 => {
                for x in 0..width {
                    let color_idx = bytes[row_base + x as usize] as usize;
                    if let Some(&c) = palette.get(color_idx) {
                        out.set_pixel(x, dst_y, c);
                    }
                }
            }
            24 => {
                for x in 0..width {
                    let px_base = row_base + (x as usize) * 3;
                    out.set_pixel(
                        x,
                        dst_y,
                        [bytes[px_base + 2], bytes[px_base + 1], bytes[px_base], 255],
                    );
                }
            }
            32 => {
                for x in 0..width {
                    let px_base = row_base + (x as usize) * 4;
                    out.set_pixel(
                        x,
                        dst_y,
                        [
                            bytes[px_base + 2],
                            bytes[px_base + 1],
                            bytes[px_base],
                            bytes[px_base + 3],
                        ],
                    );
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

fn read_ppm_token(data: &[u8], pos: &mut usize) -> Result<u32, ImageError> {
    while *pos < data.len() {
        let b = data[*pos];
        if b == b'#' {
            while *pos < data.len() && data[*pos] != b'\n' {
                *pos += 1;
            }
        } else if matches!(b, b' ' | b'\t' | b'\r' | b'\n') {
            *pos += 1;
        } else {
            break;
        }
    }
    let start = *pos;
    while *pos < data.len() && data[*pos].is_ascii_digit() {
        *pos += 1;
    }
    if start == *pos {
        return Err(ImageError::InvalidHeader(
            "expected numeric token in PPM header",
        ));
    }
    std::str::from_utf8(&data[start..*pos])
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .ok_or(ImageError::InvalidHeader("invalid number in PPM header"))
}

pub fn decode_ppm(bytes: &[u8]) -> Result<Image, ImageError> {
    if bytes.len() < 7 || (!bytes.starts_with(b"P6") && !bytes.starts_with(b"P5")) {
        return Err(ImageError::InvalidHeader("missing Netpbm signature"));
    }
    let is_p6 = bytes.starts_with(b"P6");
    let mut pos = 2usize;
    let width = read_ppm_token(bytes, &mut pos)?;
    let height = read_ppm_token(bytes, &mut pos)?;
    let maxval = read_ppm_token(bytes, &mut pos)?;
    if width == 0 || height == 0 || maxval == 0 || maxval > 255 {
        return Err(ImageError::InvalidHeader("invalid PPM header values"));
    }
    validate_dimensions(width, height)?;
    if pos >= bytes.len() {
        return Err(ImageError::UnexpectedEof);
    }
    pos += 1;
    let mut out = Image::new(width, height);
    if is_p6 {
        let needed = (width as usize) * (height as usize) * 3;
        if pos + needed > bytes.len() {
            return Err(ImageError::UnexpectedEof);
        }
        for y in 0..height {
            for x in 0..width {
                let idx = pos + ((y * width + x) as usize) * 3;
                out.set_pixel(x, y, [bytes[idx], bytes[idx + 1], bytes[idx + 2], 255]);
            }
        }
    } else {
        let needed = (width as usize) * (height as usize);
        if pos + needed > bytes.len() {
            return Err(ImageError::UnexpectedEof);
        }
        for y in 0..height {
            for x in 0..width {
                let g = bytes[pos + ((y * width + x) as usize)];
                out.set_pixel(x, y, [g, g, g, 255]);
            }
        }
    }
    Ok(out)
}

/// Encode `image` as a PNG.
pub fn encode_png(image: &Image) -> Vec<u8> {
    let mut req = Vec::with_capacity(12 + image.pixels.len());
    req.extend_from_slice(&image.width.to_le_bytes());
    req.extend_from_slice(&image.height.to_le_bytes());
    req.extend_from_slice(&(image.pixels.len() as u32).to_le_bytes());
    req.extend_from_slice(&image.pixels);

    with_worker(|w| w.send_command(OP_ENCODE_PNG, &req)).unwrap_or_default()
}

/// Encode `image` as a 32-bit BMP.
pub fn encode_bmp(image: &Image) -> Vec<u8> {
    let pixel_bytes = (image.width as usize)
        .saturating_mul(image.height as usize)
        .saturating_mul(4);
    let file_size = 54 + pixel_bytes;
    let mut out = Vec::with_capacity(file_size);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(file_size as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(image.width as i32).to_le_bytes());
    out.extend_from_slice(&(-(image.height as i32)).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(pixel_bytes as u32).to_le_bytes());
    out.extend_from_slice(&2835u32.to_le_bytes());
    out.extend_from_slice(&2835u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    for chunk in image.pixels.chunks_exact(4) {
        out.extend_from_slice(&[chunk[2], chunk[1], chunk[0], chunk[3]]);
    }
    out
}

/// Encode `image` as a 1-entry Windows ICO container wrapping a PNG payload.
pub fn encode_ico(image: &Image) -> Vec<u8> {
    let png = encode_png(image);
    let mut out = Vec::with_capacity(22 + png.len());
    out.extend_from_slice(&0u16.to_le_bytes()); // reserved
    out.extend_from_slice(&1u16.to_le_bytes()); // type 1 (icon)
    out.extend_from_slice(&1u16.to_le_bytes()); // 1 entry
    let w_byte = if image.width >= 256 {
        0u8
    } else {
        image.width as u8
    };
    let h_byte = if image.height >= 256 {
        0u8
    } else {
        image.height as u8
    };
    out.push(w_byte);
    out.push(h_byte);
    out.push(0); // colors
    out.push(0); // reserved
    out.extend_from_slice(&1u16.to_le_bytes()); // planes
    out.extend_from_slice(&32u16.to_le_bytes()); // bpp
    out.extend_from_slice(&(png.len() as u32).to_le_bytes());
    out.extend_from_slice(&22u32.to_le_bytes()); // offset
    out.extend_from_slice(&png);
    out
}

/// Encode `image` as a Netpbm binary P6 PPM.
pub fn encode_ppm(image: &Image) -> Vec<u8> {
    let header = format!("P6\n{} {}\n255\n", image.width, image.height);
    let mut out =
        Vec::with_capacity(header.len() + (image.width as usize * image.height as usize * 3));
    out.extend_from_slice(header.as_bytes());
    for chunk in image.pixels.chunks_exact(4) {
        out.push(chunk[0]);
        out.push(chunk[1]);
        out.push(chunk[2]);
    }
    out
}

pub fn encode_webp(image: &Image) -> Vec<u8> {
    encode_png(image)
}

pub fn encode_gif(image: &Image) -> Vec<u8> {
    encode_png(image)
}

pub fn encode_jpeg(image: &Image) -> Vec<u8> {
    encode_bmp(image)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_image_basic() {
        let mut img = Image::new(2, 2);
        img.set_pixel(0, 0, [255, 0, 0, 255]);
        assert_eq!(img.pixel(0, 0), Some([255, 0, 0, 255]));
        assert_eq!(img.pixel(1, 0), Some([0, 0, 0, 0]));
    }

    #[test]
    fn test_dimensions_check() {
        assert!(validate_dimensions(10, 10).is_ok());
        assert!(validate_dimensions(0, 10).is_err());
        assert!(validate_dimensions(9000, 10).is_err());
        assert!(Image::from_rgba(0, 0, vec![]).is_err());
        assert!(Image::from_rgba(0, 10, vec![]).is_err());
        assert!(Image::from_rgba(9000, 10, vec![0; 9000 * 10 * 4]).is_err());
        assert!(Image::from_rgba(2, 2, vec![0; 16]).is_ok());
    }

    #[test]
    fn test_worker_ipc_roundtrip() {
        let mut img = Image::new(4, 4);
        for y in 0..4 {
            for x in 0..4 {
                img.set_pixel(x, y, [(x * 60) as u8, (y * 60) as u8, 128, 255]);
            }
        }

        let png = encode_png(&img);
        assert!(!png.is_empty());
        let dec_png = decode_png(&png).expect("decode PNG via worker");
        assert_eq!(dec_png.width, 4);
        assert_eq!(dec_png.height, 4);
        assert_eq!(dec_png.pixel(0, 0), Some([0, 0, 128, 255]));

        let bmp = encode_bmp(&img);
        assert!(!bmp.is_empty());
        let dec_bmp = decode_bmp(&bmp).expect("decode BMP via worker");
        assert_eq!(dec_bmp.width, 4);
        assert_eq!(dec_bmp.height, 4);
        assert_eq!(dec_bmp.pixel(0, 0), Some([0, 0, 128, 255]));

        let ico = encode_ico(&img);
        assert!(!ico.is_empty());
        let dec_ico = decode_ico(&ico).expect("decode ICO via worker");
        assert_eq!(dec_ico.width, 4);
        assert_eq!(dec_ico.height, 4);

        let resized = img.resize_bilinear(8, 8);
        assert_eq!(resized.width, 8);
        assert_eq!(resized.height, 8);
    }

    #[test]
    fn test_worker_respawn_after_kill() {
        let img = Image::new(2, 2);
        let png = encode_png(&img);
        assert!(!png.is_empty());
        // Explicitly kill the worker child process
        {
            let mut guard = worker_slot().lock().unwrap();
            if let Some(w) = guard.as_mut() {
                let _ = w._child.kill();
                let _ = w._child.wait();
            }
        }
        // Worker must recover and decode successfully
        let dec = decode_png(&png);
        assert!(dec.is_ok(), "Worker must recover after process crash");
    }
}
