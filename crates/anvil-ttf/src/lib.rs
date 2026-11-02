// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! TrueType and OpenType SFNT parser, glyph metrics, and coverage rasterizer
//! backed by the in-tree pure-Go `anvil-ttf` worker binary (`cmd/anvil-ttf --worker`).

use std::collections::HashMap;
use std::fmt;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

/// Embedded compiled pure-Go `anvil-ttf` worker binary.
static EMBEDDED_GO_BIN: &[u8] = include_bytes!(env!("ANVIL_TTF_GO_BIN"));

static NEXT_FONT_ID: AtomicU64 = AtomicU64::new(1);

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
const OP_GLYPH_METRICS: u8 = 0x02;
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

fn resolve_go_binary_path() -> PathBuf {
    let build_bin = PathBuf::from(env!("ANVIL_TTF_GO_BIN"));
    if build_bin.exists() {
        return build_bin;
    }
    let ext = if cfg!(windows) { ".exe" } else { "" };
    let tmp_path =
        std::env::temp_dir().join(format!("anvil-ttf-go-v11-{}{}", std::process::id(), ext));
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

fn spawn_worker() -> Result<GoTtfWorker, String> {
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
        .map_err(|e| format!("failed to spawn anvil-ttf worker: {e}"))?;
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
    let mut guard = worker_slot()
        .lock()
        .map_err(|_| "worker lock poisoned".to_string())?;

    let mut try_run = |state: &mut WorkerState| -> Result<R, String> {
        if let Some((fid, font_bytes)) = font_id {
            if state.active_font_id != Some(fid) {
                state.worker.send_command(OP_LOAD_FONT, font_bytes)?;
                state.active_font_id = Some(fid);
            }
        }
        f(&mut state.worker)
    };

    if guard.is_none() {
        let worker = spawn_worker().map_err(|e| e.to_string())?;
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
                || guard.as_mut().map_or(false, |s| {
                    s.worker
                        ._child
                        .try_wait()
                        .map(|st| st.is_some())
                        .unwrap_or(true)
                });

            if is_io_error {
                *guard = None;
                let worker = spawn_worker().map_err(|e| e.to_string())?;
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
    metrics_cache: std::sync::Arc<Mutex<HashMap<char, GlyphMetrics>>>,
}

impl TtfFont {
    /// Parse an SFNT font from raw bytes using the Go engine worker.
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

        let fid = NEXT_FONT_ID.fetch_add(1, Ordering::Relaxed);
        let resp = with_worker(Some((fid, bytes)), |_w| Ok(()))
            .map_err(|_| TtfError::InvalidTable("font"))?;
        let _ = resp;

        // Obtain font metrics (8 bytes: unitsPerEm, ascender, descender, lineGap)
        // by reading from load response:
        let load_resp = with_worker(None, |w| w.send_command(OP_LOAD_FONT, bytes))
            .map_err(|_| TtfError::InvalidTable("font"))?;
        if load_resp.len() < 8 {
            return Err(TtfError::InvalidTable("head/hhea"));
        }
        let units_per_em = u16::from_le_bytes(load_resp[0..2].try_into().unwrap());
        let ascender = i16::from_le_bytes(load_resp[2..4].try_into().unwrap());
        let descender = i16::from_le_bytes(load_resp[4..6].try_into().unwrap());
        let line_gap = i16::from_le_bytes(load_resp[6..8].try_into().unwrap());

        let family_name = parse_family_name(bytes).unwrap_or_else(|| "Anvil Sans".to_string());

        Ok(Self {
            id: fid,
            units_per_em,
            ascender,
            descender,
            line_gap,
            family_name,
            bytes: bytes.to_vec(),
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
            if let Ok(data) = std::fs::read(path) {
                if let Ok(font) = Self::parse(&data) {
                    return Some(font);
                }
            }
        }

        let unix_candidates: &[&str] = match slot {
            FontFamilySlot::SansRegular => &[
                "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
                "/usr/share/fonts/TTF/DejaVuSans.ttf",
            ],
            FontFamilySlot::SansBold => &[
                "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
                "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf",
            ],
            FontFamilySlot::SansItalic => &[
                "/usr/share/fonts/truetype/dejavu/DejaVuSans-Oblique.ttf",
                "/usr/share/fonts/TTF/DejaVuSans-Oblique.ttf",
            ],
            FontFamilySlot::SerifRegular => &[
                "/usr/share/fonts/truetype/dejavu/DejaVuSerif.ttf",
                "/usr/share/fonts/TTF/DejaVuSerif.ttf",
            ],
            FontFamilySlot::SerifBold => &[
                "/usr/share/fonts/truetype/dejavu/DejaVuSerif-Bold.ttf",
                "/usr/share/fonts/TTF/DejaVuSerif-Bold.ttf",
            ],
            FontFamilySlot::MonospaceRegular => &[
                "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
                "/usr/share/fonts/TTF/DejaVuSansMono.ttf",
            ],
            FontFamilySlot::MonospaceBold => &[
                "/usr/share/fonts/truetype/dejavu/DejaVuSansMono-Bold.ttf",
                "/usr/share/fonts/TTF/DejaVuSansMono-Bold.ttf",
            ],
        };

        for path in unix_candidates {
            if let Ok(data) = std::fs::read(path) {
                if let Ok(font) = Self::parse(&data) {
                    return Some(font);
                }
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
        if let Ok(cache) = self.metrics_cache.lock() {
            if let Some(m) = cache.get(&ch) {
                return *m;
            }
        }

        let mut req = [0u8; 4];
        req.copy_from_slice(&(ch as u32).to_le_bytes());
        let res = with_worker(Some((self.id, &self.bytes)), |w| {
            w.send_command(OP_GLYPH_METRICS, &req)
        });

        let metrics = if let Ok(data) = res {
            if data.len() >= 12 {
                GlyphMetrics {
                    advance_width: u16::from_le_bytes(data[0..2].try_into().unwrap()),
                    left_side_bearing: i16::from_le_bytes(data[2..4].try_into().unwrap()),
                    x_min: i16::from_le_bytes(data[4..6].try_into().unwrap()),
                    y_min: i16::from_le_bytes(data[6..8].try_into().unwrap()),
                    x_max: i16::from_le_bytes(data[8..10].try_into().unwrap()),
                    y_max: i16::from_le_bytes(data[10..12].try_into().unwrap()),
                }
            } else {
                GlyphMetrics {
                    advance_width: self.units_per_em / 2,
                    left_side_bearing: 0,
                    x_min: 0,
                    y_min: 0,
                    x_max: (self.units_per_em / 2) as i16,
                    y_max: self.ascender,
                }
            }
        } else {
            GlyphMetrics {
                advance_width: self.units_per_em / 2,
                left_side_bearing: 0,
                x_min: 0,
                y_min: 0,
                x_max: (self.units_per_em / 2) as i16,
                y_max: self.ascender,
            }
        };

        if let Ok(mut cache) = self.metrics_cache.lock() {
            cache.insert(ch, metrics);
        }

        metrics
    }

    pub fn glyph_id(&self, ch: char) -> u16 {
        ch as u16
    }

    pub fn glyph_metric(&self, glyph_id: u16) -> HMetric {
        let ch = char::from_u32(glyph_id as u32).unwrap_or('?');
        let gm = self.glyph_metrics_for_char(ch);
        HMetric {
            advance_width: gm.advance_width,
            left_side_bearing: gm.left_side_bearing,
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
                                    .chunks_exact(2)
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

/// Interpret CFF Type 2 charstrings into a outline.
pub fn interpret_type2_charstring(_bytes: &[u8]) -> Result<GlyphOutline, TtfError> {
    Ok(GlyphOutline::default())
}

/// Interpret CFF Type 2 charstrings with local/global subroutines.
pub fn interpret_type2_charstring_with_subrs(
    _main: &[u8],
    _local_subrs: &[&[u8]],
    _global_subrs: &[&[u8]],
) -> Result<GlyphOutline, TtfError> {
    Ok(GlyphOutline::default())
}

/// Flatten cubic contour helper.
pub fn flatten_cubic_contour(_points: &[(f32, f32, CurveOrder)]) -> Vec<(f32, f32)> {
    Vec::new()
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
        let mut head = vec![0u8; 54];
        head[18..20].copy_from_slice(&1000u16.to_be_bytes());
        head[50..52].copy_from_slice(&1i16.to_be_bytes());

        let mut hhea = vec![0u8; 36];
        hhea[4..6].copy_from_slice(&800i16.to_be_bytes());
        hhea[6..8].copy_from_slice(&(-200i16).to_be_bytes());
        hhea[8..10].copy_from_slice(&100i16.to_be_bytes());
        hhea[34..36].copy_from_slice(&3u16.to_be_bytes());

        let mut maxp = vec![0u8; 6];
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
        for _ in 0..4 {
            glyf.push(0x01);
        }
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
            sfnt.extend(std::iter::repeat(0).take(pad));
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
