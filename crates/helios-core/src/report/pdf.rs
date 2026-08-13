//! A minimal PDF writer, just large enough for Helios's reports.
//!
//! Rendering the report through a headless browser or a PDF crate would pull
//! megabytes of dependency into an app that is otherwise auditable in an
//! afternoon. What a storage report actually needs is text in two weights,
//! horizontal rules and shaded table rows — roughly 200 lines of PDF 1.4, with
//! the two standard-14 fonts every reader already has.
//!
//! Deliberate limits: text is encoded as WinAnsi (Latin-1), so names outside
//! that range are transliterated to `?` in the PDF. CSV and JSON exports carry
//! full UTF-8 and are the right format when exact names matter.

use std::fmt::Write as _;

const PAGE_WIDTH: f32 = 595.0; // A4 at 72 dpi
const PAGE_HEIGHT: f32 = 842.0;
const MARGIN: f32 = 48.0;
const LINE_HEIGHT: f32 = 14.0;
const BODY_SIZE: f32 = 9.5;

/// Accumulates pages of drawing commands and emits a complete PDF.
#[derive(Debug)]
pub struct PdfBuilder {
    pages: Vec<String>,
    current: String,
    cursor: f32,
    title: String,
}

impl PdfBuilder {
    pub fn new(title: &str) -> PdfBuilder {
        PdfBuilder {
            pages: Vec::new(),
            current: String::new(),
            cursor: PAGE_HEIGHT - MARGIN,
            title: title.to_string(),
        }
    }

    fn ensure_space(&mut self, needed: f32) {
        if self.cursor - needed < MARGIN {
            self.page_break();
        }
    }

    pub fn page_break(&mut self) {
        if !self.current.is_empty() {
            self.pages.push(std::mem::take(&mut self.current));
        }
        self.cursor = PAGE_HEIGHT - MARGIN;
    }

    pub fn heading(&mut self, text: &str) {
        self.ensure_space(LINE_HEIGHT * 2.0);
        self.cursor -= LINE_HEIGHT * 0.6;
        self.text_at(MARGIN, self.cursor, text, 15.0, true, (0.1, 0.1, 0.12));
        self.cursor -= LINE_HEIGHT * 0.5;
        let y = self.cursor;
        self.rule(y, 0.8, (0.75, 0.75, 0.78));
        self.cursor -= LINE_HEIGHT * 0.8;
    }

    pub fn subheading(&mut self, text: &str) {
        self.ensure_space(LINE_HEIGHT * 2.0);
        self.cursor -= LINE_HEIGHT * 0.4;
        self.text_at(MARGIN, self.cursor, text, 11.0, true, (0.15, 0.15, 0.18));
        self.cursor -= LINE_HEIGHT;
    }

    pub fn paragraph(&mut self, text: &str) {
        self.ensure_space(LINE_HEIGHT);
        self.text_at(
            MARGIN,
            self.cursor,
            text,
            BODY_SIZE,
            false,
            (0.2, 0.2, 0.24),
        );
        self.cursor -= LINE_HEIGHT;
    }

    /// Draws one table row. `columns` are `(text, x_offset, right_aligned)`.
    pub fn row(&mut self, columns: &[(String, f32, bool)], bold: bool, shaded: bool) {
        self.ensure_space(LINE_HEIGHT);
        if shaded {
            let _ = write!(
                self.current,
                "0.955 0.957 0.965 rg {} {} {} {} re f",
                MARGIN - 4.0,
                self.cursor - 3.5,
                PAGE_WIDTH - MARGIN * 2.0 + 8.0,
                LINE_HEIGHT - 2.0
            );
        }
        for (text, x, right) in columns {
            let x = if *right {
                // Helvetica averages ~0.5 em per character; good enough to keep
                // a numeric column visually right-aligned.
                MARGIN + *x - text.len() as f32 * BODY_SIZE * 0.5
            } else {
                MARGIN + *x
            };
            self.text_at(x, self.cursor, text, BODY_SIZE, bold, (0.15, 0.15, 0.18));
        }
        self.cursor -= LINE_HEIGHT;
    }

    /// A proportional bar, used for the category breakdown.
    pub fn bar(&mut self, x: f32, width: f32, fraction: f32, color: (f32, f32, f32)) {
        let fill = (width * fraction.clamp(0.0, 1.0)).max(0.0);
        let _ = write!(
            self.current,
            "0.90 0.90 0.92 rg {} {} {} 7 re f\n{} {} {} rg {} {} {} 7 re f\n",
            MARGIN + x,
            self.cursor - 1.0,
            width,
            color.0,
            color.1,
            color.2,
            MARGIN + x,
            self.cursor - 1.0,
            fill
        );
    }

    pub fn spacer(&mut self, lines: f32) {
        self.cursor -= LINE_HEIGHT * lines;
    }

    fn rule(&mut self, y: f32, thickness: f32, color: (f32, f32, f32)) {
        let _ = writeln!(
            self.current,
            "{} {} {} rg {} {} {} {} re f",
            color.0,
            color.1,
            color.2,
            MARGIN,
            y,
            PAGE_WIDTH - MARGIN * 2.0,
            thickness
        );
    }

    fn text_at(&mut self, x: f32, y: f32, text: &str, size: f32, bold: bool, rgb: (f32, f32, f32)) {
        let font = if bold { "/F2" } else { "/F1" };
        let _ = writeln!(
            self.current,
            "BT {} {} {} rg {} {} Tf {:.1} {:.1} Td ({}) Tj ET",
            rgb.0,
            rgb.1,
            rgb.2,
            font,
            size,
            x,
            y,
            escape(text)
        );
    }

    /// Emits the finished document.
    pub fn finish(mut self) -> Vec<u8> {
        self.page_break();
        if self.pages.is_empty() {
            self.pages.push(String::new());
        }

        let page_count = self.pages.len();
        // Object numbering: 1 catalog, 2 pages, 3+4 fonts, then two objects per
        // page (page dict, content stream).
        let first_page_obj = 5;
        let mut objects: Vec<String> = Vec::new();

        let kids: String = (0..page_count)
            .map(|i| format!("{} 0 R ", first_page_obj + i * 2))
            .collect();

        objects.push("<< /Type /Catalog /Pages 2 0 R >>".into());
        objects.push(format!(
            "<< /Type /Pages /Kids [{}] /Count {} >>",
            kids.trim_end(),
            page_count
        ));
        objects.push(
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
                .into(),
        );
        objects.push(
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>"
                .into(),
        );

        for (i, content) in self.pages.iter().enumerate() {
            let content_obj = first_page_obj + i * 2 + 1;
            objects.push(format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {PAGE_WIDTH} {PAGE_HEIGHT}] \
                 /Resources << /Font << /F1 3 0 R /F2 4 0 R >> >> /Contents {content_obj} 0 R >>"
            ));
            objects.push(format!(
                "<< /Length {} >>\nstream\n{}endstream",
                content.len(),
                content
            ));
        }

        let mut out: Vec<u8> = Vec::with_capacity(8192);
        out.extend_from_slice(b"%PDF-1.4\n");
        // A binary comment marks the file as binary for transfer tools.
        out.extend_from_slice(b"%\xE2\xE3\xCF\xD3\n");

        let mut offsets = Vec::with_capacity(objects.len());
        for (i, body) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", i + 1, body).as_bytes());
        }

        let xref_at = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
        out.extend_from_slice(b"0000000000 65535 f \n");
        for offset in &offsets {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R /Info << /Title ({}) /Producer (Helios) >> >>\n\
                 startxref\n{}\n%%EOF\n",
                objects.len() + 1,
                escape(&self.title),
                xref_at
            )
            .as_bytes(),
        );
        out
    }
}

/// Escapes PDF string syntax and drops characters outside WinAnsi.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    for ch in text.chars() {
        match ch {
            '(' | ')' | '\\' => {
                out.push('\\');
                out.push(ch);
            }
            '\n' | '\r' | '\t' => out.push(' '),
            c if (c as u32) < 256 => out.push(c),
            _ => out.push('?'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn produces_a_structurally_valid_document() {
        let mut pdf = PdfBuilder::new("Helios Report");
        pdf.heading("Storage Summary");
        pdf.paragraph("Total capacity: 1.00 TB");
        pdf.row(
            &[("Name".into(), 0.0, false), ("Size".into(), 400.0, true)],
            true,
            false,
        );
        let bytes = pdf.finish();

        assert!(bytes.starts_with(b"%PDF-1.4"));
        assert!(bytes.ends_with(b"%%EOF\n"));
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("/Type /Catalog"));
        assert!(text.contains("startxref"));
        assert_eq!(text.matches("/Type /Page ").count(), 1);
    }

    #[test]
    fn long_reports_paginate() {
        let mut pdf = PdfBuilder::new("Long");
        for i in 0..200 {
            pdf.paragraph(&format!("row {i}"));
        }
        let text = String::from_utf8_lossy(&pdf.finish()).into_owned();
        assert!(
            text.matches("/Type /Page ").count() > 1,
            "should have spilled pages"
        );
    }

    #[test]
    fn escapes_delimiters_and_transliterates_wide_characters() {
        assert_eq!(escape("a(b)c\\d"), "a\\(b\\)c\\\\d");
        assert_eq!(escape("café"), "café");
        assert_eq!(escape("日本語"), "???");
    }
}
