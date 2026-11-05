// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! TrueType and OpenType SFNT parser, glyph metrics, and coverage rasterizer
//! backed by the in-tree pure-Go `anvil-ttf` worker binary (`cmd/anvil-ttf --worker`).

use std::collections::HashMap;
use std::fmt;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

/// Embedded compiled pure-Go `anvil-ttf` worker binary.
static EMBEDDED_GO_BIN: &[u8] = include_bytes!(env!("ANVIL_TTF_GO_BIN"));

static NEXT_FONT_ID: AtomicU64 = AtomicU64::new(1);
static WORKER_DISABLED: AtomicBool = AtomicBool::new(false);
static WORKER_EVER_SPAWNED: AtomicBool = AtomicBool::new(false);

/// Maximum font binary size accepted (`320 MiB`).
pub const MAX_FONT_BYTES: usize = 320 * 1024 * 1024;

/// Maximum number of SFNT directory tables accepted (`1,280`).
pub const MAX_SFNT_TABLES: usize = 1280;

/// Maximum contours per glyph outline (`2,560`).
pub const MAX_GLYPH_CONTOURS: usize = 2560;

/// Maximum control points per glyph outline (`40,960`).
pub const MAX_CONTOUR_POINTS: usize = 40960;

/// Maximum compound glyph recursion depth (`64`).
pub const MAX_COMPOSITE_GLYPH_DEPTH: u8 = 64;

const OP_LOAD_FONT: u8 = 0x01;
const _OP_GLYPH_METRICS: u8 = 0x02;
const OP_RASTERIZE_GLYPH: u8 = 0x03;
const OP_RASTERIZE_LCD: u8 = 0x04;
const _OP_SHUTDOWN: u8 = 0xFF;

/// Failure while reading an SFNT font.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TtfError {
    UnexpectedEof,
    FontTooLarge { size: usize },
    TooManyTables { count: usize },
    InvalidSfntSignature,
    MissingTable(&'static str),
    InvalidTable(&'static str),
}

impl fmt::Display for TtfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedEof => write!(f, "unexpected end of font file"),
            Self::FontTooLarge { size } => {
                write!(f, "font size {size} exceeds limit ({MAX_FONT_BYTES})")
            }
            Self::TooManyTables { count } => {
                write!(
                    f,
                    "SFNT table count {count} exceeds limit ({MAX_SFNT_TABLES})"
                )
            }
            Self::InvalidSfntSignature => write!(f, "unrecognized SFNT font signature"),
            Self::MissingTable(tag) => write!(f, "missing required SFNT table '{tag}'"),
            Self::InvalidTable(tag) => write!(f, "invalid SFNT table '{tag}'"),
        }
    }
}

impl std::error::Error for TtfError {}

/// Standard generic font family and style slots.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FontFamilySlot {
    SansRegular,
    SansBold,
    SansItalic,
    SerifRegular,
    SerifBold,
    MonospaceRegular,
    MonospaceBold,
}

/// Curve order for an outline point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CurveOrder {
    OnCurve,
    QuadraticOffCurve,
    CubicOffCurve,
}

/// Point on a glyph outline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutlinePoint {
    pub x: i16,
    pub y: i16,
    pub on_curve: bool,
}

impl OutlinePoint {
    pub const fn on_curve(x: i16, y: i16) -> Self {
        Self {
            x,
            y,
            on_curve: true,
        }
    }

    pub const fn quadratic(x: i16, y: i16) -> Self {
        Self {
            x,
            y,
            on_curve: false,
        }
    }

    pub const fn cubic(x: i16, y: i16) -> (i16, i16, CurveOrder) {
        (x, y, CurveOrder::CubicOffCurve)
    }
}

/// Decoded glyph outline in font design units.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GlyphOutline {
    pub x_min: i16,
    pub y_min: i16,
    pub x_max: i16,
    pub y_max: i16,
    pub contours: Vec<Vec<OutlinePoint>>,
}

/// Horizontal metric for one glyph, in font design units.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HMetric {
    pub advance_width: u16,
    pub left_side_bearing: i16,
}

/// Comprehensive glyph metrics including bounding box and bearings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GlyphMetrics {
    pub advance_width: u16,
    pub left_side_bearing: i16,
    pub x_min: i16,
    pub y_min: i16,
    pub x_max: i16,
    pub y_max: i16,
}

/// 8-bit coverage bitmap for one rasterized glyph.
#[derive(Clone, Debug, PartialEq)]
pub struct GlyphBitmap {
    pub width: u32,
    pub height: u32,
    pub bearing_x: i32,
    pub bearing_y: i32,
    pub advance_px: f32,
    pub coverage: Vec<u8>,
}

/// Alias for standard coverage mask.
pub type CoverageMask = GlyphBitmap;

/// 3x horizontal RGB subpixel LCD coverage bitmap.
#[derive(Clone, Debug, PartialEq)]
pub struct SubpixelGlyphBitmap {
    pub width: u32,
    pub height: u32,
    pub bearing_x: i32,
    pub bearing_y: i32,
    pub advance_px: f32,
    pub rgb_coverage: Vec<u8>,
}

/// Alias for standard RGB subpixel mask.
pub type RgbMask = SubpixelGlyphBitmap;

struct GoTtfWorker {
    _child: Child,
    stdin: ChildStdin,
    stdout: ChildStdout,
}

struct WorkerState {
    worker: GoTtfWorker,
    active_font_id: Option<u64>,
}

fn resolve_go_binary_path() -> Result<PathBuf, String> {
    let ext = if cfg!(windows) { ".exe" } else { "" };
    let tmp_path =
        std::env::temp_dir().join(format!("anvil-ttf-go-v11-{}{}", std::process::id(), ext));
    let needs_write = match std::fs::metadata(&tmp_path) {
        Ok(meta) => meta.len() == 0,
        Err(_) => true,
    };
    if needs_write {
        if let Err(err) = std::fs::write(&tmp_path, EMBEDDED_GO_BIN) {
            let _ = std::fs::remove_file(&tmp_path);
            WORKER_DISABLED.store(true, Ordering::Relaxed);
            return Err(format!("failed to write anvil-ttf worker binary: {err}"));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o755));
        }
    }
    Ok(tmp_path)
}

fn spawn_worker() -> Result<GoTtfWorker, String> {
    if WORKER_DISABLED.load(Ordering::Relaxed) {
        return Err("anvil-ttf worker disabled in confined process".to_string());
    }
    let bin = resolve_go_binary_path()?;
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
    let mut child = match cmd.spawn() {
        Ok(c) => {
            WORKER_EVER_SPAWNED.store(true, Ordering::Relaxed);
            c
        }
        Err(e) => {
            if !WORKER_EVER_SPAWNED.load(Ordering::Relaxed) {
                WORKER_DISABLED.store(true, Ordering::Relaxed);
            }
            return Err(format!("failed to spawn anvil-ttf worker: {e}"));
        }
    };
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "missing worker stdin".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "missing worker stdout".to_string())?;
    Ok(GoTtfWorker {
        _child: child,
        stdin,
        stdout,
    })
}

impl GoTtfWorker {
    fn send_command(&mut self, op: u8, payload: &[u8]) -> Result<Vec<u8>, String> {
        let mut req_hdr = [0u8; 5];
        req_hdr[0] = op;
        req_hdr[1..5].copy_from_slice(&(payload.len() as u32).to_le_bytes());
        self.stdin
            .write_all(&req_hdr)
            .map_err(|e| format!("worker write error: {e}"))?;
        if !payload.is_empty() {
            self.stdin
                .write_all(payload)
                .map_err(|e| format!("worker write payload error: {e}"))?;
        }
        self.stdin
            .flush()
            .map_err(|e| format!("worker flush error: {e}"))?;

        let mut resp_hdr = [0u8; 5];
        self.stdout
            .read_exact(&mut resp_hdr)
            .map_err(|e| format!("worker read error: {e}"))?;
        let status = resp_hdr[0];
        let resp_len = u32::from_le_bytes(resp_hdr[1..5].try_into().unwrap()) as usize;
        let mut resp_payload = vec![0u8; resp_len];
        if resp_len > 0 {
            self.stdout
                .read_exact(&mut resp_payload)
                .map_err(|e| format!("worker read payload error: {e}"))?;
        }
        if status != 0 {
            return Err(String::from_utf8_lossy(&resp_payload).into_owned());
        }
        Ok(resp_payload)
    }
}

fn worker_slot() -> &'static Mutex<Option<WorkerState>> {
    static SLOT: OnceLock<Mutex<Option<WorkerState>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

fn with_worker<R>(
    font_id: Option<(u64, &[u8])>,
    mut f: impl FnMut(&mut GoTtfWorker) -> Result<R, String>,
) -> Result<R, String> {
    if WORKER_DISABLED.load(Ordering::Relaxed) {
        return Err("anvil-ttf worker disabled in confined process".to_string());
    }
    let mut guard = worker_slot()
        .lock()
        .map_err(|_| "worker lock poisoned".to_string())?;

    let mut try_run = |state: &mut WorkerState| -> Result<R, String> {
        if let Some((fid, font_bytes)) = font_id
            && state.active_font_id != Some(fid)
        {
            state.worker.send_command(OP_LOAD_FONT, font_bytes)?;
            state.active_font_id = Some(fid);
        }
        f(&mut state.worker)
    };

    if guard.is_none() {
        let worker = spawn_worker()?;
        *guard = Some(WorkerState {
            worker,
            active_font_id: None,
        });
    }

    let res = if let Some(state) = guard.as_mut() {
        try_run(state)
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
                || err.contains("worker write error")
                || err.contains("worker read error")
                || err.contains("pipe")
                || err.contains("os error 232")
                || err.contains("os error 109")
                || err.contains("BrokenPipe")
                || err.contains("UnexpectedEof")
                || guard.as_mut().is_some_and(|s| {
                    s.worker
                        ._child
                        .try_wait()
                        .map(|st| st.is_some())
                        .unwrap_or(true)
                });

            if is_io_error {
                *guard = None;
                let worker = spawn_worker()?;
                let mut new_state = WorkerState {
                    worker,
                    active_font_id: None,
                };
                let retry_res = try_run(&mut new_state);
                if retry_res.is_ok() {
                    *guard = Some(new_state);
                }
                retry_res
            } else {
                Err(err)
            }
        }
    }
}

struct ParsedSfntMetrics {
    units_per_em: u16,
    ascender: i16,
    descender: i16,
    line_gap: i16,
    cmap: HashMap<char, u16>,
    hmtx: Vec<HMetric>,
}

fn read_u16_be(data: &[u8], off: usize) -> Option<u16> {
    let s = data.get(off..off + 2)?;
    Some(u16::from_be_bytes([s[0], s[1]]))
}

fn read_i16_be(data: &[u8], off: usize) -> Option<i16> {
    let s = data.get(off..off + 2)?;
    Some(i16::from_be_bytes([s[0], s[1]]))
}

fn read_u32_be(data: &[u8], off: usize) -> Option<u32> {
    let s = data.get(off..off + 4)?;
    Some(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

fn parse_cmap_format4(sub: &[u8], out: &mut HashMap<char, u16>) {
    if sub.len() < 14 {
        return;
    }
    let Some(seg_count_x2) = read_u16_be(sub, 6) else {
        return;
    };
    let seg_count = (seg_count_x2 as usize) / 2;
    if seg_count == 0 {
        return;
    }
    let end_codes = 14usize;
    let start_codes = end_codes + seg_count * 2 + 2;
    let id_deltas = start_codes + seg_count * 2;
    let id_range_offsets = id_deltas + seg_count * 2;
    if id_range_offsets + seg_count * 2 > sub.len() {
        return;
    }
    for seg in 0..seg_count {
        let (Some(end_code), Some(start_code), Some(id_delta)) = (
            read_u16_be(sub, end_codes + seg * 2),
            read_u16_be(sub, start_codes + seg * 2),
            read_i16_be(sub, id_deltas + seg * 2),
        ) else {
            break;
        };
        let id_range_pos = id_range_offsets + seg * 2;
        let Some(id_range_offset_raw) = read_u16_be(sub, id_range_pos) else {
            break;
        };
        let id_range_offset = id_range_offset_raw as usize;
        if start_code == 0xFFFF && end_code == 0xFFFF {
            break;
        }
        if start_code > end_code {
            continue;
        }
        for code in start_code..=end_code {
            let gid = if id_range_offset == 0 {
                (i32::from(code) + i32::from(id_delta)) as u16
            } else {
                let glyph_off = id_range_pos + id_range_offset + ((code - start_code) as usize) * 2;
                match read_u16_be(sub, glyph_off) {
                    Some(raw_gid) if raw_gid != 0 => {
                        (i32::from(raw_gid) + i32::from(id_delta)) as u16
                    }
                    _ => 0,
                }
            };
            if gid != 0
                && let Some(ch) = char::from_u32(u32::from(code))
            {
                out.insert(ch, gid);
            }
        }
    }
}

fn parse_cmap_format12(sub: &[u8], out: &mut HashMap<char, u16>) {
    if sub.len() < 16 {
        return;
    }
    let Some(num_groups_raw) = read_u32_be(sub, 12) else {
        return;
    };
    let num_groups = num_groups_raw as usize;
    if 16usize.saturating_add(num_groups.saturating_mul(12)) > sub.len() {
        return;
    }
    for i in 0..num_groups {
        let base = 16 + i * 12;
        let (Some(start_code), Some(end_code), Some(start_gid)) = (
            read_u32_be(sub, base),
            read_u32_be(sub, base + 4),
            read_u32_be(sub, base + 8),
        ) else {
            break;
        };
        if end_code < start_code || (end_code - start_code) > 0xFFFF {
            continue;
        }
        for code in start_code..=end_code {
            let gid = (start_gid + (code - start_code)) as u16;
            if gid != 0
                && let Some(ch) = char::from_u32(code)
            {
                out.insert(ch, gid);
            }
        }
    }
}

fn parse_sfnt_tables(bytes: &[u8]) -> Result<ParsedSfntMetrics, TtfError> {
    let num_tables = u16::from_be_bytes([bytes[4], bytes[5]]) as usize;
    let dir_end = 12usize.saturating_add(num_tables.saturating_mul(16));
    if dir_end > bytes.len() {
        return Err(TtfError::UnexpectedEof);
    }
    let mut head_slice: Option<&[u8]> = None;
    let mut hhea_slice: Option<&[u8]> = None;
    let mut maxp_slice: Option<&[u8]> = None;
    let mut hmtx_slice: Option<&[u8]> = None;
    let mut cmap_slice: Option<&[u8]> = None;

    for i in 0..num_tables {
        let rec = 12 + i * 16;
        let tag = &bytes[rec..rec + 4];
        let off = read_u32_be(bytes, rec + 8).ok_or(TtfError::UnexpectedEof)? as usize;
        let len = read_u32_be(bytes, rec + 12).ok_or(TtfError::UnexpectedEof)? as usize;
        let end = off.checked_add(len).ok_or(TtfError::UnexpectedEof)?;
        let slice = bytes.get(off..end).ok_or(TtfError::UnexpectedEof)?;
        match tag {
            b"head" => head_slice = Some(slice),
            b"hhea" => hhea_slice = Some(slice),
            b"maxp" => maxp_slice = Some(slice),
            b"hmtx" => hmtx_slice = Some(slice),
            b"cmap" => cmap_slice = Some(slice),
            _ => {}
        }
    }

    let head = head_slice.ok_or(TtfError::MissingTable("head"))?;
    if head.len() < 54 {
        return Err(TtfError::InvalidTable("head"));
    }
    let units_per_em = read_u16_be(head, 18).ok_or(TtfError::InvalidTable("head"))?;
    if units_per_em == 0 {
        return Err(TtfError::InvalidTable("head"));
    }

    let hhea = hhea_slice.ok_or(TtfError::MissingTable("hhea"))?;
    if hhea.len() < 36 {
        return Err(TtfError::InvalidTable("hhea"));
    }
    let ascender = read_i16_be(hhea, 4).ok_or(TtfError::InvalidTable("hhea"))?;
    let descender = read_i16_be(hhea, 6).ok_or(TtfError::InvalidTable("hhea"))?;
    let line_gap = read_i16_be(hhea, 8).ok_or(TtfError::InvalidTable("hhea"))?;
    let num_h_metrics = read_u16_be(hhea, 34).ok_or(TtfError::InvalidTable("hhea"))? as usize;
    if num_h_metrics == 0 {
        return Err(TtfError::InvalidTable("hhea"));
    }

    let maxp = maxp_slice.ok_or(TtfError::MissingTable("maxp"))?;
    if maxp.len() < 6 {
        return Err(TtfError::InvalidTable("maxp"));
    }
    let num_glyphs = read_u16_be(maxp, 4).ok_or(TtfError::InvalidTable("maxp"))? as usize;

    let hmtx_raw = hmtx_slice.ok_or(TtfError::MissingTable("hmtx"))?;
    if hmtx_raw.len() < num_h_metrics.saturating_mul(4) {
        return Err(TtfError::InvalidTable("hmtx"));
    }
    let total_glyphs = num_glyphs.max(num_h_metrics);
    let mut hmtx = Vec::with_capacity(total_glyphs);
    for i in 0..num_h_metrics {
        let adv = read_u16_be(hmtx_raw, i * 4).ok_or(TtfError::InvalidTable("hmtx"))?;
        let lsb = read_i16_be(hmtx_raw, i * 4 + 2).ok_or(TtfError::InvalidTable("hmtx"))?;
        hmtx.push(HMetric {
            advance_width: adv,
            left_side_bearing: lsb,
        });
    }
    let last_adv = hmtx.last().map(|m| m.advance_width).unwrap_or(0);
    for i in num_h_metrics..total_glyphs {
        let lsb_off = num_h_metrics * 4 + (i - num_h_metrics) * 2;
        let lsb = read_i16_be(hmtx_raw, lsb_off).unwrap_or(0);
        hmtx.push(HMetric {
            advance_width: last_adv,
            left_side_bearing: lsb,
        });
    }

    let mut cmap = HashMap::new();
    if let Some(cmap_tbl) = cmap_slice
        && cmap_tbl.len() >= 4
        && let Some(num_subtables) = read_u16_be(cmap_tbl, 2)
    {
        let n = num_subtables as usize;
        if 4 + n * 8 <= cmap_tbl.len() {
            for i in 0..n {
                let rec = 4 + i * 8;
                let (Some(platform_id), Some(encoding_id), Some(sub_off_raw)) = (
                    read_u16_be(cmap_tbl, rec),
                    read_u16_be(cmap_tbl, rec + 2),
                    read_u32_be(cmap_tbl, rec + 4),
                ) else {
                    continue;
                };
                if platform_id != 0
                    && !(platform_id == 3 && (encoding_id == 1 || encoding_id == 10))
                {
                    continue;
                }
                let sub_off = sub_off_raw as usize;
                let Some(sub) = cmap_tbl.get(sub_off..) else {
                    continue;
                };
                let Some(format) = read_u16_be(sub, 0) else {
                    continue;
                };
                match format {
                    4 => parse_cmap_format4(sub, &mut cmap),
                    12 => parse_cmap_format12(sub, &mut cmap),
                    _ => {}
                }
            }
        }
    }

    Ok(ParsedSfntMetrics {
        units_per_em,
        ascender,
        descender,
        line_gap,
        cmap,
        hmtx,
    })
}

/// Parsed TrueType or OpenType font handle.
#[derive(Clone, Debug)]
pub struct TtfFont {
    pub id: u64,
    pub units_per_em: u16,
    pub ascender: i16,
    pub descender: i16,
    pub line_gap: i16,
    family_name: String,
    bytes: Vec<u8>,
    cmap: std::sync::Arc<HashMap<char, u16>>,
    hmtx: std::sync::Arc<Vec<HMetric>>,
    metrics_cache: std::sync::Arc<Mutex<HashMap<char, GlyphMetrics>>>,
}

impl TtfFont {
    /// Parse an SFNT font from raw bytes.
    pub fn parse(bytes: &[u8]) -> Result<Self, TtfError> {
        if bytes.len() > MAX_FONT_BYTES {
            return Err(TtfError::FontTooLarge { size: bytes.len() });
        }
        if bytes.len() < 12 {
            return Err(TtfError::UnexpectedEof);
        }
        let sig = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        if sig != 0x0001_0000 && sig != 0x7472_7565 && sig != 0x4F54_544F {
            return Err(TtfError::InvalidSfntSignature);
        }
        let num_tables = u16::from_be_bytes([bytes[4], bytes[5]]) as usize;
        if num_tables > MAX_SFNT_TABLES {
            return Err(TtfError::TooManyTables { count: num_tables });
        }

        let sfnt = parse_sfnt_tables(bytes)?;
        let fid = NEXT_FONT_ID.fetch_add(1, Ordering::Relaxed);
        let family_name = parse_family_name(bytes).unwrap_or_else(|| "Anvil Sans".to_string());

        Ok(Self {
            id: fid,
            units_per_em: sfnt.units_per_em,
            ascender: sfnt.ascender,
            descender: sfnt.descender,
            line_gap: sfnt.line_gap,
            family_name,
            bytes: bytes.to_vec(),
            cmap: std::sync::Arc::new(sfnt.cmap),
            hmtx: std::sync::Arc::new(sfnt.hmtx),
            metrics_cache: std::sync::Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub fn family_name(&self) -> &str {
        &self.family_name
    }

    /// Load standard system font slot.
    pub fn load_system_font_slot(slot: FontFamilySlot) -> Option<Self> {
        let win_candidates: &[&str] = match slot {
            FontFamilySlot::SansRegular => &[
                r"C:\Windows\Fonts\segoeui.ttf",
                r"C:\Windows\Fonts\arial.ttf",
            ],
            FontFamilySlot::SansBold => &[
                r"C:\Windows\Fonts\segoeuib.ttf",
                r"C:\Windows\Fonts\arialbd.ttf",
            ],
            FontFamilySlot::SansItalic => &[
                r"C:\Windows\Fonts\segoeuii.ttf",
                r"C:\Windows\Fonts\ariali.ttf",
            ],
            FontFamilySlot::SerifRegular => &[
                r"C:\Windows\Fonts\georgia.ttf",
                r"C:\Windows\Fonts\times.ttf",
            ],
            FontFamilySlot::SerifBold => &[
                r"C:\Windows\Fonts\georgiab.ttf",
                r"C:\Windows\Fonts\timesbd.ttf",
            ],
            FontFamilySlot::MonospaceRegular => &[
                r"C:\Windows\Fonts\consola.ttf",
                r"C:\Windows\Fonts\cour.ttf",
            ],
            FontFamilySlot::MonospaceBold => &[
                r"C:\Windows\Fonts\consolab.ttf",
                r"C:\Windows\Fonts\courbd.ttf",
            ],
        };

        for path in win_candidates {
            if let Ok(data) = std::fs::read(path)
                && let Ok(font) = Self::parse(&data)
            {
                return Some(font);
            }
        }

        let unix_candidates: &[&str] = match slot {
            FontFamilySlot::SansRegular => &[
                "/usr/share/fonts/truetype/noto-sans-ttf/NotoSans-Regular.ttf",
                "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
                "/usr/share/fonts/noto/NotoSans-Regular.ttf",
                "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
                "/usr/share/fonts/TTF/DejaVuSans.ttf",
                "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
                "/usr/share/fonts/liberation/LiberationSans-Regular.ttf",
                "/usr/share/fonts/truetype/urw-base35/NimbusSans-Regular.ttf",
            ],
            FontFamilySlot::SansBold => &[
                "/usr/share/fonts/truetype/noto-sans-ttf/NotoSans-Bold.ttf",
                "/usr/share/fonts/truetype/noto/NotoSans-Bold.ttf",
                "/usr/share/fonts/noto/NotoSans-Bold.ttf",
                "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
                "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf",
                "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
                "/usr/share/fonts/liberation/LiberationSans-Bold.ttf",
                "/usr/share/fonts/truetype/urw-base35/NimbusSans-Bold.ttf",
            ],
            FontFamilySlot::SansItalic => &[
                "/usr/share/fonts/truetype/noto-sans-ttf/NotoSans-Italic.ttf",
                "/usr/share/fonts/truetype/noto/NotoSans-Italic.ttf",
                "/usr/share/fonts/noto/NotoSans-Italic.ttf",
                "/usr/share/fonts/truetype/dejavu/DejaVuSans-Oblique.ttf",
                "/usr/share/fonts/TTF/DejaVuSans-Oblique.ttf",
                "/usr/share/fonts/truetype/liberation/LiberationSans-Italic.ttf",
                "/usr/share/fonts/liberation/LiberationSans-Italic.ttf",
                "/usr/share/fonts/truetype/urw-base35/NimbusSans-Italic.ttf",
            ],
            FontFamilySlot::SerifRegular => &[
                "/usr/share/fonts/truetype/noto-sans-ttf/NotoSerif-Regular.ttf",
                "/usr/share/fonts/truetype/noto/NotoSerif-Regular.ttf",
                "/usr/share/fonts/noto/NotoSerif-Regular.ttf",
                "/usr/share/fonts/truetype/dejavu/DejaVuSerif.ttf",
                "/usr/share/fonts/TTF/DejaVuSerif.ttf",
                "/usr/share/fonts/truetype/liberation/LiberationSerif-Regular.ttf",
                "/usr/share/fonts/liberation/LiberationSerif-Regular.ttf",
                "/usr/share/fonts/truetype/urw-base35/NimbusRoman-Regular.ttf",
            ],
            FontFamilySlot::SerifBold => &[
                "/usr/share/fonts/truetype/noto-sans-ttf/NotoSerif-Bold.ttf",
                "/usr/share/fonts/truetype/noto/NotoSerif-Bold.ttf",
                "/usr/share/fonts/noto/NotoSerif-Bold.ttf",
                "/usr/share/fonts/truetype/dejavu/DejaVuSerif-Bold.ttf",
                "/usr/share/fonts/TTF/DejaVuSerif-Bold.ttf",
                "/usr/share/fonts/truetype/liberation/LiberationSerif-Bold.ttf",
                "/usr/share/fonts/liberation/LiberationSerif-Bold.ttf",
                "/usr/share/fonts/truetype/urw-base35/NimbusRoman-Bold.ttf",
            ],
            FontFamilySlot::MonospaceRegular => &[
                "/usr/share/fonts/truetype/hack/Hack-Regular.ttf",
                "/usr/share/fonts/TTF/Hack-Regular.ttf",
                "/usr/share/fonts/truetype/noto-sans-ttf/NotoSansMono-Regular.ttf",
                "/usr/share/fonts/truetype/noto/NotoSansMono-Regular.ttf",
                "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
                "/usr/share/fonts/TTF/DejaVuSansMono.ttf",
                "/usr/share/fonts/truetype/liberation/LiberationMono-Regular.ttf",
                "/usr/share/fonts/liberation/LiberationMono-Regular.ttf",
            ],
            FontFamilySlot::MonospaceBold => &[
                "/usr/share/fonts/truetype/hack/Hack-Bold.ttf",
                "/usr/share/fonts/TTF/Hack-Bold.ttf",
                "/usr/share/fonts/truetype/noto-sans-ttf/NotoSansMono-Bold.ttf",
                "/usr/share/fonts/truetype/noto/NotoSansMono-Bold.ttf",
                "/usr/share/fonts/truetype/dejavu/DejaVuSansMono-Bold.ttf",
                "/usr/share/fonts/TTF/DejaVuSansMono-Bold.ttf",
                "/usr/share/fonts/truetype/liberation/LiberationMono-Bold.ttf",
                "/usr/share/fonts/liberation/LiberationMono-Bold.ttf",
            ],
        };

        for path in unix_candidates {
            if let Ok(data) = std::fs::read(path)
                && let Ok(font) = Self::parse(&data)
            {
                return Some(font);
            }
        }

        None
    }

    /// Load standard system font for given bold and monospace flags.
    pub fn load_system_font(bold: bool, monospace: bool) -> Option<Self> {
        let slot = match (monospace, bold) {
            (true, true) => FontFamilySlot::MonospaceBold,
            (true, false) => FontFamilySlot::MonospaceRegular,
            (false, true) => FontFamilySlot::SansBold,
            (false, false) => FontFamilySlot::SansRegular,
        };
        Self::load_system_font_slot(slot)
    }

    /// Load system font by family name.
    pub fn load_system_font_by_family(family_name: &str) -> Option<Self> {
        let lower = family_name.to_ascii_lowercase();
        if lower.contains("mono") || lower.contains("consol") {
            Self::load_system_font_slot(FontFamilySlot::MonospaceRegular)
        } else if lower.contains("serif") || lower.contains("times") || lower.contains("georgia") {
            Self::load_system_font_slot(FontFamilySlot::SerifRegular)
        } else {
            Self::load_system_font_slot(FontFamilySlot::SansRegular)
        }
    }

    /// Look up full glyph metrics for character `ch`.
    pub fn glyph_metrics_for_char(&self, ch: char) -> GlyphMetrics {
        if let Ok(cache) = self.metrics_cache.lock()
            && let Some(m) = cache.get(&ch)
        {
            return *m;
        }

        let gid = self.glyph_id(ch);
        let hm = self.glyph_metric(gid);
        let metrics = GlyphMetrics {
            advance_width: hm.advance_width,
            left_side_bearing: hm.left_side_bearing,
            x_min: hm.left_side_bearing,
            y_min: 0,
            x_max: hm
                .left_side_bearing
                .saturating_add(hm.advance_width.min(i16::MAX as u16) as i16),
            y_max: self.ascender,
        };

        if let Ok(mut cache) = self.metrics_cache.lock() {
            cache.insert(ch, metrics);
        }

        metrics
    }

    pub fn glyph_id(&self, ch: char) -> u16 {
        self.cmap.get(&ch).copied().unwrap_or(0)
    }

    pub fn glyph_metric(&self, glyph_id: u16) -> HMetric {
        if let Some(m) = self.hmtx.get(glyph_id as usize) {
            *m
        } else if let Some(first) = self.hmtx.first() {
            *first
        } else {
            HMetric {
                advance_width: self.units_per_em / 2,
                left_side_bearing: 0,
            }
        }
    }

    pub fn kerning_units(&self, _left_glyph: u16, _right_glyph: u16) -> i16 {
        0
    }

    pub fn glyph_advance_px(&self, ch: char, font_size_px: f32) -> f32 {
        let gm = self.glyph_metrics_for_char(ch);
        (gm.advance_width as f32) * font_size_px / (self.units_per_em as f32)
    }

    pub fn measure_text(&self, text: &str, font_size_px: f32) -> f32 {
        let mut total = 0.0f32;
        for ch in text.chars() {
            total += self.glyph_advance_px(ch, font_size_px);
        }
        total
    }

    pub fn line_height_px(&self, font_size_px: f32) -> f32 {
        let units = i32::from(self.ascender) - i32::from(self.descender) + i32::from(self.line_gap);
        (units.max(i32::from(self.units_per_em)) as f32) * font_size_px / (self.units_per_em as f32)
    }

    pub fn glyph_outline(&self, _glyph_id: u16) -> Option<&GlyphOutline> {
        None
    }

    /// Rasterize glyph `ch` at `font_size_px` with 8-bit coverage.
    pub fn rasterize_glyph(&self, ch: char, font_size_px: f32) -> Option<GlyphBitmap> {
        self.rasterize_glyph_styled(ch, font_size_px, 0, false, false)
    }

    /// Rasterize glyph `ch` with 3x horizontal LCD subpixel coverage.
    pub fn rasterize_glyph_subpixel_lcd(
        &self,
        ch: char,
        font_size_px: f32,
    ) -> Option<SubpixelGlyphBitmap> {
        let mut req = [0u8; 8];
        req[0..4].copy_from_slice(&(ch as u32).to_le_bytes());
        req[4..8].copy_from_slice(&font_size_px.to_bits().to_le_bytes());

        let res = with_worker(Some((self.id, &self.bytes)), |w| {
            w.send_command(OP_RASTERIZE_LCD, &req)
        })
        .ok()?;

        if res.len() < 20 {
            return None;
        }

        let width = u32::from_le_bytes(res[0..4].try_into().unwrap());
        let height = u32::from_le_bytes(res[4..8].try_into().unwrap());
        let bearing_x = i32::from_le_bytes(res[8..12].try_into().unwrap());
        let bearing_y = i32::from_le_bytes(res[12..16].try_into().unwrap());
        let advance_px = f32::from_bits(u32::from_le_bytes(res[16..20].try_into().unwrap()));
        let rgb_coverage = res[20..].to_vec();

        Some(SubpixelGlyphBitmap {
            width,
            height,
            bearing_x,
            bearing_y,
            advance_px,
            rgb_coverage,
        })
    }

    pub fn rasterize_glyph_subpixel_positioned(
        &self,
        ch: char,
        font_size_px: f32,
        phase: u8,
    ) -> Option<GlyphBitmap> {
        self.rasterize_glyph_styled(ch, font_size_px, phase, false, false)
    }

    /// Rasterize glyph `ch` with styled attributes.
    pub fn rasterize_glyph_styled(
        &self,
        ch: char,
        font_size_px: f32,
        phase: u8,
        bold: bool,
        italic: bool,
    ) -> Option<GlyphBitmap> {
        let mut req = [0u8; 11];
        req[0..4].copy_from_slice(&(ch as u32).to_le_bytes());
        req[4..8].copy_from_slice(&font_size_px.to_bits().to_le_bytes());
        req[8] = phase;
        req[9] = u8::from(bold);
        req[10] = u8::from(italic);

        let res = with_worker(Some((self.id, &self.bytes)), |w| {
            w.send_command(OP_RASTERIZE_GLYPH, &req)
        })
        .ok()?;

        if res.len() < 20 {
            return None;
        }

        let width = u32::from_le_bytes(res[0..4].try_into().unwrap());
        let height = u32::from_le_bytes(res[4..8].try_into().unwrap());
        let bearing_x = i32::from_le_bytes(res[8..12].try_into().unwrap());
        let bearing_y = i32::from_le_bytes(res[12..16].try_into().unwrap());
        let advance_px = f32::from_bits(u32::from_le_bytes(res[16..20].try_into().unwrap()));
        let coverage = res[20..].to_vec();

        Some(GlyphBitmap {
            width,
            height,
            bearing_x,
            bearing_y,
            advance_px,
            coverage,
        })
    }
}

fn parse_family_name(bytes: &[u8]) -> Option<String> {
    if bytes.len() < 12 {
        return None;
    }
    let num_tables = u16::from_be_bytes([bytes[4], bytes[5]]) as usize;
    for i in 0..num_tables {
        let rec = 12 + i * 16;
        if rec + 16 > bytes.len() {
            break;
        }
        if &bytes[rec..rec + 4] == b"name" {
            let offset = u32::from_be_bytes(bytes[rec + 8..rec + 12].try_into().unwrap()) as usize;
            let length = u32::from_be_bytes(bytes[rec + 12..rec + 16].try_into().unwrap()) as usize;
            if offset + length <= bytes.len() && length >= 6 {
                let name_tbl = &bytes[offset..offset + length];
                let count = u16::from_be_bytes([name_tbl[2], name_tbl[3]]) as usize;
                let str_off = u16::from_be_bytes([name_tbl[4], name_tbl[5]]) as usize;
                for j in 0..count {
                    let r = 6 + j * 12;
                    if r + 12 > name_tbl.len() {
                        break;
                    }
                    let name_id = u16::from_be_bytes([name_tbl[r + 6], name_tbl[r + 7]]);
                    if name_id == 1 {
                        let len = u16::from_be_bytes([name_tbl[r + 8], name_tbl[r + 9]]) as usize;
                        let off = str_off
                            + (u16::from_be_bytes([name_tbl[r + 10], name_tbl[r + 11]]) as usize);
                        if off + len <= name_tbl.len() {
                            let raw = &name_tbl[off..off + len];
                            if raw.len() >= 2 && raw[0] == 0 {
                                let u16s: Vec<u16> = raw
                                    .as_chunks::<2>()
                                    .0
                                    .iter()
                                    .map(|c| u16::from_be_bytes([c[0], c[1]]))
                                    .collect();
                                return String::from_utf16(&u16s).ok();
                            }
                            return String::from_utf8(raw.to_vec()).ok();
                        }
                    }
                }
            }
        }
    }
    None
}

/// Estimate a string width from proportional em ratios when no font is loaded.
pub fn measure_proportional_fallback(text: &str, font_size_px: f32) -> f32 {
    let mut total: f32 = 0.0;
    for ch in text.chars() {
        let ratio = match ch {
            ' ' => 0.30,
            '.' | ',' | ':' | ';' | '!' | '|' | '\'' | '`' | 'i' | 'l' => 0.28,
            'I' | 'j' | 'f' | 't' | 'r' | '(' | ')' | '[' | ']' | '{' | '}' => 0.36,
            'w' | 'm' | 'M' | 'W' => 0.85,
            '@' | '%' | '&' => 0.80,
            _ => 0.55,
        };
        total += ratio * font_size_px;
    }
    total
}

/// Convert an 8-bit sRGB channel to 12-bit linear light.
#[inline]
pub fn srgb_to_linear_u12(srgb: u8) -> u16 {
    static LUT: OnceLock<[u16; 256]> = OnceLock::new();
    let table = LUT.get_or_init(|| {
        let mut t = [0u16; 256];
        for (i, slot) in t.iter_mut().enumerate() {
            let s = (i as f32) / 255.0;
            let lin = if s <= 0.04045 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            };
            *slot = (lin * 4095.0).round().clamp(0.0, 4095.0) as u16;
        }
        t
    });
    unsafe { *table.get_unchecked(srgb as usize) }
}

/// Convert a 12-bit linear light value back to 8-bit sRGB.
#[inline]
pub fn linear_u12_to_srgb(linear: u16) -> u8 {
    static LUT: OnceLock<[u8; 4096]> = OnceLock::new();
    let table = LUT.get_or_init(|| {
        let mut t = [0u8; 4096];
        for (i, slot) in t.iter_mut().enumerate() {
            let lin = (i as f32) / 4095.0;
            let s = if lin <= 0.0031308 {
                lin * 12.92
            } else {
                1.055 * lin.powf(1.0 / 2.4) - 0.055
            };
            *slot = (s * 255.0).round().clamp(0.0, 255.0) as u8;
        }
        t
    });
    let idx = (linear as usize).min(4095);
    unsafe { *table.get_unchecked(idx) }
}

/// Stem contrast enhancement curve.
#[inline]
pub fn enhance_stem_contrast(coverage: u8, fg_luma: u8) -> u8 {
    if coverage == 0 || coverage == 255 {
        return coverage;
    }
    let a = u32::from(coverage);
    let inv = 255 - a;
    let boost = (a * inv) / 255;
    let dark_weight = u32::from(255 - fg_luma);
    let delta = (boost * (dark_weight + 64)) / 512;
    (a + delta).min(255) as u8
}

/// Gamma-correct linear space blending for single coverage channel.
#[inline]
pub fn blend_glyph_pixel_gamma_correct(
    dst_rgb: (u8, u8, u8),
    fg_rgb: (u8, u8, u8),
    coverage: u8,
) -> (u8, u8, u8) {
    if coverage == 0 {
        return dst_rgb;
    }
    if coverage == 255 {
        return fg_rgb;
    }
    let fg_luma =
        ((u32::from(fg_rgb.0) * 54 + u32::from(fg_rgb.1) * 183 + u32::from(fg_rgb.2) * 19) >> 8)
            as u8;
    let alpha = u32::from(enhance_stem_contrast(coverage, fg_luma));
    let inv_alpha = 255 - alpha;

    let dr = u32::from(srgb_to_linear_u12(dst_rgb.0));
    let dg = u32::from(srgb_to_linear_u12(dst_rgb.1));
    let db = u32::from(srgb_to_linear_u12(dst_rgb.2));

    let sr = u32::from(srgb_to_linear_u12(fg_rgb.0));
    let sg = u32::from(srgb_to_linear_u12(fg_rgb.1));
    let sb = u32::from(srgb_to_linear_u12(fg_rgb.2));

    let or = ((sr * alpha + dr * inv_alpha + 127) / 255) as u16;
    let og = ((sg * alpha + dg * inv_alpha + 127) / 255) as u16;
    let ob = ((sb * alpha + db * inv_alpha + 127) / 255) as u16;

    (
        linear_u12_to_srgb(or),
        linear_u12_to_srgb(og),
        linear_u12_to_srgb(ob),
    )
}

/// Gamma-correct linear space blending for 3-channel RGB subpixel LCD coverage.
#[inline]
pub fn blend_subpixel_lcd_gamma_correct(
    dst_rgb: (u8, u8, u8),
    fg_rgb: (u8, u8, u8),
    cov_rgb: (u8, u8, u8),
) -> (u8, u8, u8) {
    if cov_rgb == (0, 0, 0) {
        return dst_rgb;
    }
    if cov_rgb == (255, 255, 255) {
        return fg_rgb;
    }
    let fg_luma =
        ((u32::from(fg_rgb.0) * 54 + u32::from(fg_rgb.1) * 183 + u32::from(fg_rgb.2) * 19) >> 8)
            as u8;
    let ar = u32::from(enhance_stem_contrast(cov_rgb.0, fg_luma));
    let ag = u32::from(enhance_stem_contrast(cov_rgb.1, fg_luma));
    let ab = u32::from(enhance_stem_contrast(cov_rgb.2, fg_luma));

    let dr = u32::from(srgb_to_linear_u12(dst_rgb.0));
    let dg = u32::from(srgb_to_linear_u12(dst_rgb.1));
    let db = u32::from(srgb_to_linear_u12(dst_rgb.2));

    let sr = u32::from(srgb_to_linear_u12(fg_rgb.0));
    let sg = u32::from(srgb_to_linear_u12(fg_rgb.1));
    let sb = u32::from(srgb_to_linear_u12(fg_rgb.2));

    let or = ((sr * ar + dr * (255 - ar) + 127) / 255) as u16;
    let og = ((sg * ag + dg * (255 - ag) + 127) / 255) as u16;
    let ob = ((sb * ab + db * (255 - ab) + 127) / 255) as u16;

    (
        linear_u12_to_srgb(or),
        linear_u12_to_srgb(og),
        linear_u12_to_srgb(ob),
    )
}

fn cff_subr_bias(count: usize) -> i32 {
    if count < 1240 {
        107
    } else if count < 33900 {
        1131
    } else {
        32768
    }
}

fn flush_cff_contour(
    current: &mut Vec<(f32, f32, CurveOrder)>,
    contours: &mut Vec<Vec<OutlinePoint>>,
) {
    if current.len() >= 2 && contours.len() < MAX_GLYPH_CONTOURS {
        let poly = flatten_cubic_contour(current);
        if poly.len() >= 3 {
            let max_pts = poly.len().min(MAX_CONTOUR_POINTS);
            let pts: Vec<OutlinePoint> = poly[..max_pts]
                .iter()
                .map(|&(x, y)| OutlinePoint {
                    x: x.round().clamp(-32768.0, 32767.0) as i16,
                    y: y.round().clamp(-32768.0, 32767.0) as i16,
                    on_curve: true,
                })
                .collect();
            contours.push(pts);
        }
    }
    current.clear();
}

/// Interpret CFF Type 2 charstrings into a outline.
pub fn interpret_type2_charstring(bytes: &[u8]) -> Result<GlyphOutline, TtfError> {
    interpret_type2_charstring_with_subrs(bytes, &[], &[])
}

/// Interpret CFF Type 2 charstrings with local/global subroutines.
pub fn interpret_type2_charstring_with_subrs(
    main: &[u8],
    local_subrs: &[&[u8]],
    global_subrs: &[&[u8]],
) -> Result<GlyphOutline, TtfError> {
    let mut stack: Vec<f32> = Vec::with_capacity(48);
    let mut contours: Vec<Vec<OutlinePoint>> = Vec::new();
    let mut current: Vec<(f32, f32, CurveOrder)> = Vec::new();

    let mut cx = 0.0_f32;
    let mut cy = 0.0_f32;
    let mut stem_hints = 0_usize;
    let mut program = main;
    let mut i = 0_usize;
    let mut call_stack: Vec<(&[u8], usize)> = Vec::new();

    let local_bias = cff_subr_bias(local_subrs.len());
    let global_bias = cff_subr_bias(global_subrs.len());

    loop {
        if i >= program.len() {
            if let Some((prev_prog, prev_idx)) = call_stack.pop() {
                program = prev_prog;
                i = prev_idx;
                continue;
            }
            break;
        }

        let b0 = program[i];
        i += 1;

        match b0 {
            28 => {
                if i + 2 > program.len() {
                    break;
                }
                let val = i16::from_be_bytes([program[i], program[i + 1]]) as f32;
                i += 2;
                if stack.len() < 48 {
                    stack.push(val);
                }
            }
            32..=246 => {
                if stack.len() < 48 {
                    stack.push((i32::from(b0) - 139) as f32);
                }
            }
            247..=250 => {
                if i >= program.len() {
                    break;
                }
                let b1 = program[i];
                i += 1;
                let val = ((i32::from(b0) - 247) * 256 + i32::from(b1) + 108) as f32;
                if stack.len() < 48 {
                    stack.push(val);
                }
            }
            251..=254 => {
                if i >= program.len() {
                    break;
                }
                let b1 = program[i];
                i += 1;
                let val = -(((i32::from(b0) - 251) * 256 + i32::from(b1) + 108) as f32);
                if stack.len() < 48 {
                    stack.push(val);
                }
            }
            255 => {
                if i + 4 > program.len() {
                    break;
                }
                let raw = i32::from_be_bytes([
                    program[i],
                    program[i + 1],
                    program[i + 2],
                    program[i + 3],
                ]);
                i += 4;
                if stack.len() < 48 {
                    stack.push((raw as f32) / 65536.0);
                }
            }
            1 | 3 | 18 | 23 => {
                stem_hints += stack.len() / 2;
                stack.clear();
            }
            19 | 20 => {
                stem_hints += stack.len() / 2;
                stack.clear();
                let mask_bytes = stem_hints.div_ceil(8);
                i = (i + mask_bytes).min(program.len());
            }
            21 => {
                flush_cff_contour(&mut current, &mut contours);
                if stack.len() >= 2 {
                    cx += stack[stack.len() - 2];
                    cy += stack[stack.len() - 1];
                    current.push((cx, cy, CurveOrder::OnCurve));
                }
                stack.clear();
            }
            22 => {
                flush_cff_contour(&mut current, &mut contours);
                if let Some(&dx) = stack.last() {
                    cx += dx;
                    current.push((cx, cy, CurveOrder::OnCurve));
                }
                stack.clear();
            }
            4 => {
                flush_cff_contour(&mut current, &mut contours);
                if let Some(&dy) = stack.last() {
                    cy += dy;
                    current.push((cx, cy, CurveOrder::OnCurve));
                }
                stack.clear();
            }
            5 => {
                let mut p = 0;
                while p + 1 < stack.len() {
                    cx += stack[p];
                    cy += stack[p + 1];
                    current.push((cx, cy, CurveOrder::OnCurve));
                    p += 2;
                }
                stack.clear();
            }
            6 | 7 => {
                let mut horiz = b0 == 6;
                for &delta in &stack {
                    if horiz {
                        cx += delta;
                    } else {
                        cy += delta;
                    }
                    current.push((cx, cy, CurveOrder::OnCurve));
                    horiz = !horiz;
                }
                stack.clear();
            }
            8 => {
                let mut seg = 0;
                while seg + 5 < stack.len() {
                    let x1 = cx + stack[seg];
                    let y1 = cy + stack[seg + 1];
                    let x2 = x1 + stack[seg + 2];
                    let y2 = y1 + stack[seg + 3];
                    cx = x2 + stack[seg + 4];
                    cy = y2 + stack[seg + 5];
                    current.push((x1, y1, CurveOrder::CubicOffCurve));
                    current.push((x2, y2, CurveOrder::CubicOffCurve));
                    current.push((cx, cy, CurveOrder::OnCurve));
                    seg += 6;
                }
                stack.clear();
            }
            24 => {
                if stack.len() >= 8 {
                    let curve_end = stack.len() - 2;
                    let mut seg = 0;
                    while seg + 5 < curve_end {
                        let x1 = cx + stack[seg];
                        let y1 = cy + stack[seg + 1];
                        let x2 = x1 + stack[seg + 2];
                        let y2 = y1 + stack[seg + 3];
                        cx = x2 + stack[seg + 4];
                        cy = y2 + stack[seg + 5];
                        current.push((x1, y1, CurveOrder::CubicOffCurve));
                        current.push((x2, y2, CurveOrder::CubicOffCurve));
                        current.push((cx, cy, CurveOrder::OnCurve));
                        seg += 6;
                    }
                    cx += stack[curve_end];
                    cy += stack[curve_end + 1];
                    current.push((cx, cy, CurveOrder::OnCurve));
                }
                stack.clear();
            }
            25 => {
                if stack.len() >= 8 {
                    let line_end = stack.len() - 6;
                    let mut p = 0;
                    while p + 1 < line_end {
                        cx += stack[p];
                        cy += stack[p + 1];
                        current.push((cx, cy, CurveOrder::OnCurve));
                        p += 2;
                    }
                    let seg = &stack[line_end..];
                    let x1 = cx + seg[0];
                    let y1 = cy + seg[1];
                    let x2 = x1 + seg[2];
                    let y2 = y1 + seg[3];
                    cx = x2 + seg[4];
                    cy = y2 + seg[5];
                    current.push((x1, y1, CurveOrder::CubicOffCurve));
                    current.push((x2, y2, CurveOrder::CubicOffCurve));
                    current.push((cx, cy, CurveOrder::OnCurve));
                }
                stack.clear();
            }
            26 => {
                let mut idx = 0;
                let mut dx1 = 0.0_f32;
                if stack.len() % 4 == 1 {
                    dx1 = stack[0];
                    idx = 1;
                }
                while idx + 3 < stack.len() {
                    let x1 = cx + dx1;
                    let y1 = cy + stack[idx];
                    let x2 = x1 + stack[idx + 1];
                    let y2 = y1 + stack[idx + 2];
                    cx = x2;
                    cy = y2 + stack[idx + 3];
                    current.push((x1, y1, CurveOrder::CubicOffCurve));
                    current.push((x2, y2, CurveOrder::CubicOffCurve));
                    current.push((cx, cy, CurveOrder::OnCurve));
                    dx1 = 0.0;
                    idx += 4;
                }
                stack.clear();
            }
            27 => {
                let mut idx = 0;
                let mut dy1 = 0.0_f32;
                if stack.len() % 4 == 1 {
                    dy1 = stack[0];
                    idx = 1;
                }
                while idx + 3 < stack.len() {
                    let x1 = cx + stack[idx];
                    let y1 = cy + dy1;
                    let x2 = x1 + stack[idx + 1];
                    let y2 = y1 + stack[idx + 2];
                    cx = x2 + stack[idx + 3];
                    cy = y2;
                    current.push((x1, y1, CurveOrder::CubicOffCurve));
                    current.push((x2, y2, CurveOrder::CubicOffCurve));
                    current.push((cx, cy, CurveOrder::OnCurve));
                    dy1 = 0.0;
                    idx += 4;
                }
                stack.clear();
            }
            30 | 31 => {
                let mut vert_first = b0 == 30;
                let mut idx = 0;
                while idx + 3 < stack.len() {
                    let trailing = if idx + 5 == stack.len() {
                        stack[idx + 4]
                    } else {
                        0.0
                    };
                    if vert_first {
                        let x1 = cx;
                        let y1 = cy + stack[idx];
                        let x2 = x1 + stack[idx + 1];
                        let y2 = y1 + stack[idx + 2];
                        cx = x2 + stack[idx + 3];
                        cy = y2 + trailing;
                        current.push((x1, y1, CurveOrder::CubicOffCurve));
                        current.push((x2, y2, CurveOrder::CubicOffCurve));
                        current.push((cx, cy, CurveOrder::OnCurve));
                    } else {
                        let x1 = cx + stack[idx];
                        let y1 = cy;
                        let x2 = x1 + stack[idx + 1];
                        let y2 = y1 + stack[idx + 2];
                        cx = x2 + trailing;
                        cy = y2 + stack[idx + 3];
                        current.push((x1, y1, CurveOrder::CubicOffCurve));
                        current.push((x2, y2, CurveOrder::CubicOffCurve));
                        current.push((cx, cy, CurveOrder::OnCurve));
                    }
                    vert_first = !vert_first;
                    idx += if idx + 5 == stack.len() { 5 } else { 4 };
                }
                stack.clear();
            }
            12 => {
                if i < program.len() {
                    i += 1;
                }
                stack.clear();
            }
            10 | 29 => {
                if let Some(biased) = stack.pop() {
                    let (subrs, bias) = if b0 == 29 {
                        (global_subrs, global_bias)
                    } else {
                        (local_subrs, local_bias)
                    };
                    let subr_idx = (biased.round() as i32) + bias;
                    if subr_idx >= 0 && (subr_idx as usize) < subrs.len() && call_stack.len() < 16 {
                        call_stack.push((program, i));
                        program = subrs[subr_idx as usize];
                        i = 0;
                    }
                }
            }
            11 => {
                i = program.len();
            }
            14 => {
                flush_cff_contour(&mut current, &mut contours);
                break;
            }
            _ => {
                stack.clear();
            }
        }
    }

    flush_cff_contour(&mut current, &mut contours);
    if contours.is_empty() {
        return Ok(GlyphOutline::default());
    }

    let mut x_min = i16::MAX;
    let mut y_min = i16::MAX;
    let mut x_max = i16::MIN;
    let mut y_max = i16::MIN;
    for contour in &contours {
        for pt in contour {
            x_min = x_min.min(pt.x);
            y_min = y_min.min(pt.y);
            x_max = x_max.max(pt.x);
            y_max = y_max.max(pt.y);
        }
    }

    Ok(GlyphOutline {
        x_min,
        y_min,
        x_max,
        y_max,
        contours,
    })
}

fn flatten_quad_bezier(
    p0: (f32, f32),
    c: (f32, f32),
    p2: (f32, f32),
    steps: usize,
    out: &mut Vec<(f32, f32)>,
) {
    let steps = steps.max(1);
    let f_steps = steps as f32;
    for s in 0..steps {
        let t = (s as f32) / f_steps;
        let inv = 1.0 - t;
        let x = inv * inv * p0.0 + 2.0 * inv * t * c.0 + t * t * p2.0;
        let y = inv * inv * p0.1 + 2.0 * inv * t * c.1 + t * t * p2.1;
        out.push((x, y));
    }
}

fn flatten_cubic_bezier(
    p0: (f32, f32),
    c1: (f32, f32),
    c2: (f32, f32),
    p3: (f32, f32),
    steps: usize,
    out: &mut Vec<(f32, f32)>,
) {
    let steps = steps.max(1);
    let f_steps = steps as f32;
    for s in 0..steps {
        let t = (s as f32) / f_steps;
        let inv = 1.0 - t;
        let b0 = inv * inv * inv;
        let b1 = 3.0 * inv * inv * t;
        let b2 = 3.0 * inv * t * t;
        let b3 = t * t * t;
        let x = b0 * p0.0 + b1 * c1.0 + b2 * c2.0 + b3 * p3.0;
        let y = b0 * p0.1 + b1 * c1.1 + b2 * c2.1 + b3 * p3.1;
        out.push((x, y));
    }
}

/// Flatten cubic contour helper.
pub fn flatten_cubic_contour(points: &[(f32, f32, CurveOrder)]) -> Vec<(f32, f32)> {
    if points.len() < 2 {
        return Vec::new();
    }

    let first_on = points
        .iter()
        .position(|p| p.2 == CurveOrder::OnCurve)
        .unwrap_or(0);

    let n = points.len();
    let mut poly = Vec::new();
    let mut idx = 0;

    while idx < n {
        let p0 = points[(first_on + idx) % n];
        let p1 = points[(first_on + idx + 1) % n];

        match p1.2 {
            CurveOrder::OnCurve => {
                poly.push((p0.0, p0.1));
                idx += 1;
            }
            CurveOrder::QuadraticOffCurve => {
                let p2 = points[(first_on + idx + 2) % n];
                flatten_quad_bezier((p0.0, p0.1), (p1.0, p1.1), (p2.0, p2.1), 6, &mut poly);
                idx += 2;
            }
            CurveOrder::CubicOffCurve => {
                let c2 = points[(first_on + idx + 2) % n];
                let p3 = points[(first_on + idx + 3) % n];
                flatten_cubic_bezier(
                    (p0.0, p0.1),
                    (p1.0, p1.1),
                    (c2.0, c2.1),
                    (p3.0, p3.1),
                    8,
                    &mut poly,
                );
                idx += 3;
            }
        }
    }

    poly
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fallback_measure() {
        let w = measure_proportional_fallback("Hello World", 16.0);
        assert!(w > 0.0);
    }

    #[test]
    fn test_gamma_conversions() {
        let lin = srgb_to_linear_u12(128);
        assert!(lin > 0 && lin < 4095);
        let srgb = linear_u12_to_srgb(lin);
        assert!((i32::from(srgb) - 128).abs() <= 1);
    }

    fn build_minimal_synthetic_ttf() -> Vec<u8> {
        let mut head = [0u8; 54];
        head[18..20].copy_from_slice(&1000u16.to_be_bytes());
        head[50..52].copy_from_slice(&1i16.to_be_bytes());

        let mut hhea = [0u8; 36];
        hhea[4..6].copy_from_slice(&800i16.to_be_bytes());
        hhea[6..8].copy_from_slice(&(-200i16).to_be_bytes());
        hhea[8..10].copy_from_slice(&100i16.to_be_bytes());
        hhea[34..36].copy_from_slice(&3u16.to_be_bytes());

        let mut maxp = [0u8; 6];
        maxp[4..6].copy_from_slice(&3u16.to_be_bytes());

        let mut hmtx = Vec::new();
        for &(adv, lsb) in &[(500u16, 0i16), (700u16, 20i16), (300u16, 10i16)] {
            hmtx.extend_from_slice(&adv.to_be_bytes());
            hmtx.extend_from_slice(&lsb.to_be_bytes());
        }

        let mut glyf = Vec::new();
        glyf.extend_from_slice(&1i16.to_be_bytes());
        glyf.extend_from_slice(&0i16.to_be_bytes());
        glyf.extend_from_slice(&0i16.to_be_bytes());
        glyf.extend_from_slice(&600i16.to_be_bytes());
        glyf.extend_from_slice(&700i16.to_be_bytes());
        glyf.extend_from_slice(&3u16.to_be_bytes());
        glyf.extend_from_slice(&0u16.to_be_bytes());
        glyf.extend_from_slice(&[0x01; 4]);
        for dx in &[0i16, 600, 0, -600] {
            glyf.extend_from_slice(&dx.to_be_bytes());
        }
        for dy in &[0i16, 0, 700, -700] {
            glyf.extend_from_slice(&dy.to_be_bytes());
        }
        let g1_len = glyf.len() as u32;

        let mut loca = Vec::new();
        loca.extend_from_slice(&0u32.to_be_bytes());
        loca.extend_from_slice(&0u32.to_be_bytes());
        loca.extend_from_slice(&g1_len.to_be_bytes());
        loca.extend_from_slice(&g1_len.to_be_bytes());

        let mut cmap = Vec::new();
        cmap.extend_from_slice(&0u16.to_be_bytes());
        cmap.extend_from_slice(&1u16.to_be_bytes());
        cmap.extend_from_slice(&3u16.to_be_bytes());
        cmap.extend_from_slice(&10u16.to_be_bytes());
        cmap.extend_from_slice(&12u32.to_be_bytes());

        cmap.extend_from_slice(&12u16.to_be_bytes());
        cmap.extend_from_slice(&0u16.to_be_bytes());
        cmap.extend_from_slice(&40u32.to_be_bytes());
        cmap.extend_from_slice(&0u32.to_be_bytes());
        cmap.extend_from_slice(&2u32.to_be_bytes());
        cmap.extend_from_slice(&65u32.to_be_bytes());
        cmap.extend_from_slice(&65u32.to_be_bytes());
        cmap.extend_from_slice(&1u32.to_be_bytes());
        cmap.extend_from_slice(&105u32.to_be_bytes());
        cmap.extend_from_slice(&105u32.to_be_bytes());
        cmap.extend_from_slice(&2u32.to_be_bytes());

        let name_tbl = vec![
            0, 0, 0, 1, 0, 18, 0, 3, 0, 1, 0, 0, 0, 1, 0, 8, 0, 0, 0, b'T', 0, b'e', 0, b's', 0,
            b't',
        ];

        let tbls = [
            (b"cmap", &cmap[..]),
            (b"glyf", &glyf[..]),
            (b"head", &head[..]),
            (b"hhea", &hhea[..]),
            (b"hmtx", &hmtx[..]),
            (b"loca", &loca[..]),
            (b"maxp", &maxp[..]),
            (b"name", &name_tbl[..]),
        ];

        let mut sfnt = Vec::new();
        sfnt.extend_from_slice(&0x00010000u32.to_be_bytes());
        sfnt.extend_from_slice(&(tbls.len() as u16).to_be_bytes());
        sfnt.extend_from_slice(&128u16.to_be_bytes());
        sfnt.extend_from_slice(&3u16.to_be_bytes());
        sfnt.extend_from_slice(&0u16.to_be_bytes());

        let mut offset = 12 + tbls.len() * 16;
        for (tag, data) in &tbls {
            sfnt.extend_from_slice(*tag);
            sfnt.extend_from_slice(&0u32.to_be_bytes());
            sfnt.extend_from_slice(&(offset as u32).to_be_bytes());
            sfnt.extend_from_slice(&(data.len() as u32).to_be_bytes());
            offset += (data.len() + 3) & !3;
        }

        for (_, data) in &tbls {
            sfnt.extend_from_slice(data);
            let pad = (4 - (data.len() % 4)) % 4;
            sfnt.extend(std::iter::repeat_n(0, pad));
        }

        sfnt
    }

    #[test]
    fn test_worker_ipc_parse_and_rasterize() {
        let ttf = build_minimal_synthetic_ttf();
        let font = TtfFont::parse(&ttf).expect("parse synthetic font");
        assert_eq!(font.units_per_em, 1000);
        assert_eq!(font.ascender, 800);
        assert_eq!(font.descender, -200);

        let adv = font.glyph_advance_px('A', 20.0);
        assert!(adv > 0.0);

        let measured = font.measure_text("AAA", 20.0);
        assert!((measured - adv * 3.0).abs() < 1e-3);

        let bmp = font.rasterize_glyph('A', 20.0).expect("rasterize A");
        assert!(bmp.width > 0 && bmp.height > 0);
        assert!(bmp.coverage.iter().any(|&c| c > 0));

        let lcd = font
            .rasterize_glyph_subpixel_lcd('A', 20.0)
            .expect("rasterize LCD A");
        assert!(lcd.width > 0 && lcd.height > 0);
        assert_eq!(
            lcd.rgb_coverage.len(),
            (lcd.width * lcd.height * 3) as usize
        );
        assert!(lcd.rgb_coverage.iter().any(|&c| c > 0));
    }

    #[test]
    fn test_worker_respawn_after_kill() {
        let ttf = build_minimal_synthetic_ttf();
        let font = TtfFont::parse(&ttf).expect("parse synthetic font");
        let adv1 = font.glyph_advance_px('A', 20.0);
        assert!(adv1 > 0.0);
        let bmp1 = font.rasterize_glyph('A', 20.0).expect("initial rasterize");
        assert!(bmp1.width > 0 && bmp1.height > 0);

        // Explicitly kill the worker child process
        {
            let mut guard = worker_slot().lock().unwrap();
            if let Some(s) = guard.as_mut() {
                let _ = s.worker._child.kill();
                let _ = s.worker._child.wait();
            }
        }

        // Worker must recover, reload the font, and rasterize successfully
        let bmp = font
            .rasterize_glyph('A', 20.0)
            .expect("rasterize after crash recovery");
        assert!(bmp.width > 0 && bmp.height > 0);
    }
}
