// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Save a page as a PDF file.
//!
//! The page is drawn with `anvil-raster` one sheet at a time, and each sheet
//! goes into the file as one picture. Text is not selectable in the file, but
//! what you get is exactly what the window draws. The writer is plain Rust:
//! a header, one object per piece, a cross-reference table, and a trailer.
//! Pictures use the `RunLengthDecode` filter, which every PDF reader knows
//! and which keeps flat page backgrounds small.

use anvil_paint::DisplayList;
use anvil_raster::{Color, Framebuffer, rasterize};

/// Plain white, for the blank sheet in an empty file.
const WHITE: Color = Color {
    r: 255,
    g: 255,
    b: 255,
    a: 255,
};

/// Sheet shape: 8.5 by 11, so a sheet is `width * 22 / 17` pixels tall.
const SHEET_RATIO: f32 = 11.0 / 8.5;
/// Most sheets one file holds. A longer page is cut off after this many.
pub const MAX_PDF_SHEETS: usize = 40;
/// Largest file we write, so it always fits as one download.
pub const MAX_PDF_BYTES: usize = 48 * 1024 * 1024;
/// Pixels are drawn at 96 per inch and PDF points are 72 per inch.
const POINTS_PER_PIXEL: f32 = 0.75;

/// A PDF file being built from sheets.
#[derive(Debug, Default)]
pub struct PdfWriter {
    /// Image and content streams, two per sheet, already encoded.
    sheets: Vec<Sheet>,
    /// Bytes the sheets will take, used to stop before the size cap.
    approx_bytes: usize,
}

#[derive(Debug)]
struct Sheet {
    width_px: u32,
    height_px: u32,
    image: Vec<u8>,
}

impl PdfWriter {
    /// Start an empty file.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sheets added so far.
    pub fn sheet_count(&self) -> usize {
        self.sheets.len()
    }

    /// Add one sheet from a framebuffer. Returns `false`, and adds nothing,
    /// when the file already holds [`MAX_PDF_SHEETS`] or the sheet would push
    /// it past [`MAX_PDF_BYTES`].
    pub fn add_sheet(&mut self, fb: &Framebuffer) -> bool {
        if self.sheets.len() >= MAX_PDF_SHEETS || fb.width() == 0 || fb.height() == 0 {
            return false;
        }
        let image = run_length_encode_rgb(fb.pixels());
        if self.approx_bytes.saturating_add(image.len()) > MAX_PDF_BYTES {
            return false;
        }
        self.approx_bytes += image.len() + 512;
        self.sheets.push(Sheet {
            width_px: fb.width(),
            height_px: fb.height(),
            image,
        });
        true
    }

    /// Write the whole file. A file with no sheets gets one blank white sheet,
    /// so readers always open it.
    pub fn finish(mut self) -> Vec<u8> {
        if self.sheets.is_empty() {
            let blank = Framebuffer::new(816, 1056, WHITE);
            self.add_sheet(&blank);
        }
        let mut out = Vec::with_capacity(self.approx_bytes + 1024);
        let mut offsets: Vec<usize> = Vec::new();
        out.extend_from_slice(b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n");

        // Object 1 is the catalog and object 2 the page tree. Each sheet then
        // takes three objects: page, picture, and drawing commands.
        let sheet_obj = |i: usize| 3 + i * 3;
        offsets.push(out.len());
        out.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
        offsets.push(out.len());
        let kids: Vec<String> = (0..self.sheets.len())
            .map(|i| format!("{} 0 R", sheet_obj(i)))
            .collect();
        out.extend_from_slice(
            format!(
                "2 0 obj\n<< /Type /Pages /Kids [{}] /Count {} >>\nendobj\n",
                kids.join(" "),
                self.sheets.len()
            )
            .as_bytes(),
        );
        for (i, sheet) in self.sheets.iter().enumerate() {
            let page_id = sheet_obj(i);
            let image_id = page_id + 1;
            let content_id = page_id + 2;
            let w_pt = points(sheet.width_px);
            let h_pt = points(sheet.height_px);
            offsets.push(out.len());
            out.extend_from_slice(
                format!(
                    "{page_id} 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w_pt} {h_pt}] \
                     /Resources << /XObject << /Im0 {image_id} 0 R >> >> /Contents {content_id} 0 R >>\nendobj\n"
                )
                .as_bytes(),
            );
            offsets.push(out.len());
            out.extend_from_slice(
                format!(
                    "{image_id} 0 obj\n<< /Type /XObject /Subtype /Image /Width {} /Height {} \
                     /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /RunLengthDecode /Length {} >>\nstream\n",
                    sheet.width_px,
                    sheet.height_px,
                    sheet.image.len()
                )
                .as_bytes(),
            );
            out.extend_from_slice(&sheet.image);
            out.extend_from_slice(b"\nendstream\nendobj\n");
            let draw = format!("q {w_pt} 0 0 {h_pt} 0 0 cm /Im0 Do Q\n");
            offsets.push(out.len());
            out.extend_from_slice(
                format!(
                    "{content_id} 0 obj\n<< /Length {} >>\nstream\n{draw}endstream\nendobj\n",
                    draw.len()
                )
                .as_bytes(),
            );
        }
        let xref_at = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n", offsets.len() + 1).as_bytes());
        out.extend_from_slice(b"0000000000 65535 f \n");
        for offset in &offsets {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF\n",
                offsets.len() + 1
            )
            .as_bytes(),
        );
        out
    }
}

/// Pixel count as PDF points, without a trailing `.0` for whole numbers.
fn points(px: u32) -> String {
    let pt = px as f32 * POINTS_PER_PIXEL;
    if pt.fract() == 0.0 {
        format!("{}", pt as u32)
    } else {
        format!("{pt:.2}")
    }
}

/// Sheet height in pixels for a page drawn `width` pixels wide.
pub fn sheet_height_for(width: u32) -> u32 {
    ((width.max(1) as f32) * SHEET_RATIO).round().max(1.0) as u32
}

/// Draw `list` as sheets `width` pixels wide and write them as a PDF.
/// `page_height` is how tall the page is; at least one sheet is written.
pub fn display_list_to_pdf(list: &DisplayList, width: u32, page_height: f32) -> Vec<u8> {
    let width = width.max(1);
    let sheet_h = sheet_height_for(width);
    let total = page_height.max(1.0);
    let mut writer = PdfWriter::new();
    let mut top = 0.0_f32;
    while top < total {
        let fb = rasterize(list, width, sheet_h, top);
        if !writer.add_sheet(&fb) {
            break;
        }
        top += sheet_h as f32;
    }
    writer.finish()
}

/// Encode top-down RGBA pixels as RGB bytes with the PDF `RunLengthDecode`
/// scheme: a length byte `n` below 128 means copy the next `n + 1` bytes,
/// above 128 means repeat the next byte `257 - n` times, and 128 ends it.
pub fn run_length_encode_rgb(rgba: &[u8]) -> Vec<u8> {
    let rgb: Vec<u8> = rgba
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|px| [px[0], px[1], px[2]])
        .collect();
    let mut out = Vec::with_capacity(rgb.len() / 4 + 16);
    let mut i = 0;
    while i < rgb.len() {
        let mut run = 1;
        while i + run < rgb.len() && run < 128 && rgb[i + run] == rgb[i] {
            run += 1;
        }
        if run >= 2 {
            out.push((257 - run) as u8);
            out.push(rgb[i]);
            i += run;
            continue;
        }
        let start = i;
        let mut len = 0;
        while i < rgb.len() && len < 128 {
            if i + 1 < rgb.len() && rgb[i + 1] == rgb[i] {
                break;
            }
            i += 1;
            len += 1;
        }
        if len == 0 {
            // A repeat starts here; let the run branch take it.
            continue;
        }
        out.push((len - 1) as u8);
        out.extend_from_slice(&rgb[start..start + len]);
    }
    out.push(128);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color { r, g, b, a: 255 }
    }

    /// Undo [`run_length_encode_rgb`] the way a PDF reader does.
    fn decode(data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < data.len() {
            let n = data[i];
            i += 1;
            match n {
                128 => break,
                0..=127 => {
                    let len = n as usize + 1;
                    out.extend_from_slice(&data[i..i + len]);
                    i += len;
                }
                _ => {
                    let len = 257 - n as usize;
                    out.extend(std::iter::repeat_n(data[i], len));
                    i += 1;
                }
            }
        }
        out
    }

    #[test]
    /// Run-length bytes decode back to the same RGB, for flat and noisy rows.
    fn run_length_round_trips() {
        let mut rgba = Vec::new();
        for i in 0..1000u32 {
            let v = if i < 400 { 7 } else { (i * 37 % 251) as u8 };
            rgba.extend_from_slice(&[v, v.wrapping_add(1), v, 255]);
        }
        let encoded = run_length_encode_rgb(&rgba);
        let rgb: Vec<u8> = rgba
            .chunks(4)
            .flat_map(|px| [px[0], px[1], px[2]])
            .collect();
        assert_eq!(decode(&encoded), rgb);
        assert_eq!(*encoded.last().unwrap(), 128);
    }

    #[test]
    /// A flat white sheet shrinks to a small fraction of its raw size.
    fn flat_sheets_stay_small() {
        let fb = Framebuffer::new(816, 1056, WHITE);
        let encoded = run_length_encode_rgb(fb.pixels());
        assert!(encoded.len() < 816 * 1056 * 3 / 50);
    }

    #[test]
    /// The file has a header, one page per sheet, a cross-reference table
    /// whose offsets land on each object, and a trailer.
    fn writer_lays_out_a_readable_file() {
        let mut writer = PdfWriter::new();
        let fb = Framebuffer::new(100, 129, rgb(10, 20, 30));
        assert!(writer.add_sheet(&fb));
        assert!(writer.add_sheet(&fb));
        let bytes = writer.finish();
        assert!(bytes.starts_with(b"%PDF-1.4\n"));
        assert!(bytes.ends_with(b"%%EOF\n"));
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("/Count 2"));
        assert_eq!(text.matches("/Type /Page ").count(), 2);
        assert!(text.contains("/MediaBox [0 0 75 96.75]"));
        // Offsets count bytes, and the pictures are not text, so read the
        // table from the raw bytes.
        let tail_at = bytes
            .windows(10)
            .rposition(|w| w == b"startxref\n")
            .expect("startxref");
        let tail = std::str::from_utf8(&bytes[tail_at + 10..]).expect("ascii trailer");
        let xref_at: usize = tail.lines().next().unwrap().parse().expect("offset");
        let table = std::str::from_utf8(&bytes[xref_at..]).expect("ascii table");
        assert!(table.starts_with("xref\n0 9\n"));
        for (n, line) in table.lines().skip(3).take(8).enumerate() {
            let offset: usize = line[..10].parse().expect("offset");
            let want = format!("{} 0 obj", n + 1);
            assert!(
                bytes[offset..].starts_with(want.as_bytes()),
                "object {} is not at {offset}",
                n + 1
            );
        }
    }

    #[test]
    /// A file with nothing in it still gets one blank sheet.
    fn empty_writer_still_writes_one_sheet() {
        let text = String::from_utf8_lossy(&PdfWriter::new().finish()).into_owned();
        assert!(text.contains("/Count 1"));
    }

    #[test]
    /// The sheet cap stops a very long page.
    fn sheet_cap_holds() {
        let mut writer = PdfWriter::new();
        let fb = Framebuffer::new(4, 5, rgb(0, 0, 0));
        for _ in 0..MAX_PDF_SHEETS {
            assert!(writer.add_sheet(&fb));
        }
        assert!(!writer.add_sheet(&fb));
        assert_eq!(writer.sheet_count(), MAX_PDF_SHEETS);
    }

    #[test]
    /// A page three sheets tall comes out as three sheets.
    fn long_page_is_cut_into_sheets() {
        let list = DisplayList::default();
        let width = 200;
        let tall = sheet_height_for(width) as f32 * 2.5;
        let text = String::from_utf8_lossy(&display_list_to_pdf(&list, width, tall)).into_owned();
        assert!(text.contains("/Count 3"));
    }
}
