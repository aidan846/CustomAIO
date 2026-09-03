//! Frame rendering.
//!
//! The layout of every style is written against a 320x320 design grid and
//! scaled to whatever the device actually wants, so a style needs no changes to
//! work on a different panel size. The pixmap and the parsed fonts are
//! allocated once and reused for every frame.

use ab_glyph::{Font, FontVec, PxScale, ScaleFont};
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Rect, Transform};

use crate::config::{color, Colors, Options, Style};

/// The design grid every style is laid out against.
const GRID: f32 = 320.0;

pub struct Renderer {
    regular: FontVec,
    bold: FontVec,
    /// Reused frame buffer, sized to the target device.
    pixmap: Pixmap,
    /// Scratch buffer for rotation, allocated only when rotation is used.
    rotated: Option<Pixmap>,
    scale: f32,
    size: u32,
}

/// What a style is handed each frame.
pub struct Frame<'a> {
    pub cpu: Option<f32>,
    pub gpu: Option<f32>,
    pub colors: &'a Colors,
    pub opts: &'a Options,
}

impl Renderer {
    pub fn new(opts: &Options, size: u32) -> Result<Self, String> {
        let load = |path: &str, what: &str| -> Result<FontVec, String> {
            let bytes = std::fs::read(path)
                .map_err(|e| format!("could not read {what} font {path}: {e}"))?;
            FontVec::try_from_vec(bytes).map_err(|e| format!("invalid font {path}: {e}"))
        };
        // Fall back to the regular face if only one of the two is present.
        let bold = load(&opts.font_bold, "bold")
            .or_else(|_| load(&opts.font, "regular"))?;
        let regular = load(&opts.font, "regular")
            .or_else(|_| load(&opts.font_bold, "bold"))?;

        let pixmap = Pixmap::new(size, size)
            .ok_or_else(|| format!("could not allocate a {size}x{size} frame"))?;
        Ok(Renderer {
            regular,
            bold,
            pixmap,
            rotated: None,
            scale: size as f32 / GRID,
            size,
        })
    }

    /// Render one frame and return the finished pixmap.
    pub fn draw(&mut self, style: &Style, cpu: Option<f32>, gpu: Option<f32>, rotation: u32)
        -> &Pixmap
    {
        let frame = Frame { cpu, gpu, colors: &style.colors, opts: &style.options };

        let alerting = frame.opts.alert_enabled && overheating(&frame).is_some();
        let bg = if alerting { &frame.colors.alert_background } else { &frame.colors.background };
        self.pixmap.fill(color(bg));

        if alerting {
            self.draw_alert(&frame);
        } else {
            match style.name.as_str() {
                "stacked" => self.draw_stacked(&frame),
                "dial" => self.draw_dial(&frame),
                "minimal" => self.draw_minimal(&frame),
                // Unknown names fall back to the original layout rather than
                // rendering a blank screen.
                _ => self.draw_classic(&frame),
            }
        }

        self.apply_rotation(rotation)
    }

    // --------------------------------------------------------
    // Styles
    // --------------------------------------------------------

    /// The original Python layout: label, big number, horizontal bar, for CPU
    /// on the top half and GPU on the bottom.
    fn draw_classic(&mut self, f: &Frame) {
        let mut rows: Vec<(&str, Option<f32>, &str)> = Vec::new();
        if f.opts.show_cpu {
            rows.push(("CPU", f.cpu, &f.colors.accent));
        }
        if f.opts.show_gpu {
            rows.push(("GPU", f.gpu, &f.colors.accent_gpu));
        }
        if rows.is_empty() {
            return;
        }

        // Rows are spaced by a fixed 135px rather than splitting the panel in
        // half. An even split puts the second row 25px lower, which pushes its
        // scale labels against the bottom edge; 135 is the original spacing.
        let single = rows.len() == 1;
        for (i, (label, temp, accent)) in rows.into_iter().enumerate() {
            // A lone row is centred instead of sitting at the top.
            let top = if single { 75.0 } else { i as f32 * 135.0 };
            if f.opts.show_labels {
                self.text_centered(label, top + 30.0, 18.0, &f.colors.text, false);
            }
            let readout = self.readout_color(f, temp, accent);
            self.text_centered(&temp_text(temp, f.opts), top + 50.0, 48.0, &readout, true);
            if f.opts.show_bars {
                self.temp_bar(top + 112.0, temp, f, accent);
            }
        }
    }

    /// Two large numbers stacked with their labels beside them, no bars.
    fn draw_stacked(&mut self, f: &Frame) {
        let mut rows: Vec<(&str, Option<f32>, &str)> = Vec::new();
        if f.opts.show_cpu {
            rows.push(("CPU", f.cpu, &f.colors.accent));
        }
        if f.opts.show_gpu {
            rows.push(("GPU", f.gpu, &f.colors.accent_gpu));
        }
        if rows.is_empty() {
            return;
        }
        let block = GRID / rows.len() as f32;
        for (i, (label, temp, accent)) in rows.into_iter().enumerate() {
            let mid = i as f32 * block + block / 2.0;
            let readout = self.readout_color(f, temp, accent);
            self.text_centered(&temp_text(temp, f.opts), mid - 42.0, 68.0, &readout, true);
            if f.opts.show_labels {
                self.text_centered(label, mid + 30.0, 20.0, &f.colors.text, false);
            }
        }
    }

    /// Circular gauges side by side.
    fn draw_dial(&mut self, f: &Frame) {
        let mut cols: Vec<(&str, Option<f32>, &str)> = Vec::new();
        if f.opts.show_cpu {
            cols.push(("CPU", f.cpu, &f.colors.accent));
        }
        if f.opts.show_gpu {
            cols.push(("GPU", f.gpu, &f.colors.accent_gpu));
        }
        if cols.is_empty() {
            return;
        }

        let n = cols.len() as f32;
        // A single dial gets the middle of the screen and a larger radius.
        let radius = if n > 1.0 { 68.0 } else { 105.0 };
        for (i, (label, temp, accent)) in cols.into_iter().enumerate() {
            let cx = GRID * (i as f32 + 0.5) / n;
            let cy = GRID / 2.0;
            let width = radius * 0.22;

            if f.opts.show_bars {
                // 270 degrees of sweep, opening at the bottom.
                self.arc(cx, cy, radius, width, 135.0, 405.0, &f.colors.track);
                if let Some(t) = temp {
                    let frac = fraction(t, f.opts);
                    if frac > 0.0 {
                        self.arc(cx, cy, radius, width, 135.0, 135.0 + 270.0 * frac, accent);
                    }
                }
            }
            let readout = self.readout_color(f, temp, accent);
            let font_size = radius * 0.52;
            self.text_at(&temp_text(temp, f.opts), cx, cy - font_size * 0.62, font_size, &readout, true);
            if f.opts.show_labels {
                self.text_at(label, cx, cy + radius * 0.42, radius * 0.20, &f.colors.text, false);
            }
        }
    }

    /// Just the numbers.
    fn draw_minimal(&mut self, f: &Frame) {
        let mut rows: Vec<(Option<f32>, &str)> = Vec::new();
        if f.opts.show_cpu {
            rows.push((f.cpu, &f.colors.accent));
        }
        if f.opts.show_gpu {
            rows.push((f.gpu, &f.colors.accent_gpu));
        }
        if rows.is_empty() {
            return;
        }
        let block = GRID / rows.len() as f32;
        let size = if rows.len() > 1 { 86.0 } else { 130.0 };
        for (i, (temp, accent)) in rows.into_iter().enumerate() {
            let mid = i as f32 * block + block / 2.0;
            let readout = self.readout_color(f, temp, accent);
            self.text_centered(&temp_text(temp, f.opts), mid - size * 0.62, size, &readout, true);
        }
    }

    fn draw_alert(&mut self, f: &Frame) {
        let who = overheating(f).unwrap_or("CPU");
        self.text_centered(&format!("{who} IS"), 90.0, 28.0, &f.colors.alert_text, true);
        self.text_centered("TOO HOT!!!", 130.0, 28.0, &f.colors.alert_text, true);
        self.text_centered("Give it a break.", 190.0, 22.0, &f.colors.alert_text, true);
    }

    // --------------------------------------------------------
    // Drawing helpers. All coordinates are in the 320x320 grid.
    // --------------------------------------------------------

    /// The pill-shaped bar used by the classic style, with optional end labels.
    fn temp_bar(&mut self, y: f32, temp: Option<f32>, f: &Frame, accent: &str) {
        let (x1, x2, h) = (75.0f32, 245.0f32, 14.0f32);
        let r = h / 2.0;
        self.round_rect(x1, y, x2 - x1, h, r, &f.colors.track);
        if let Some(t) = temp {
            let frac = fraction(t, f.opts);
            if frac > 0.0 {
                // Never draw narrower than the cap radius, or the rounded
                // rectangle collapses into a sliver.
                let w = ((x2 - x1) * frac).max(h);
                self.round_rect(x1, y, w, h, r, accent);
            }
        }
        if f.opts.show_scale {
            let ty = y + h + 4.0;
            self.text_left(&format!("{}\u{00B0}C", f.opts.bar_min as i32), x1, ty, 11.0, &f.colors.text);
            self.text_right(&format!("{}\u{00B0}C", f.opts.bar_max as i32), x2, ty, 11.0, &f.colors.text);
        }
    }

    fn round_rect(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, fill: &str) {
        let s = self.scale;
        let (x, y, w, h, r) = (x * s, y * s, w * s, h * s, r * s);
        let rect = match Rect::from_xywh(x, y, w, h) {
            Some(r) => r,
            None => return,
        };
        let mut pb = PathBuilder::new();
        let r = r.min(w / 2.0).min(h / 2.0);
        if r <= 0.0 {
            pb.push_rect(rect);
        } else {
            // Rounded rectangle from four quarter-circle arcs.
            let (l, t, right, b) = (x, y, x + w, y + h);
            let k = r * 0.5523; // circle approximation constant for cubics
            pb.move_to(l + r, t);
            pb.line_to(right - r, t);
            pb.cubic_to(right - r + k, t, right, t + r - k, right, t + r);
            pb.line_to(right, b - r);
            pb.cubic_to(right, b - r + k, right - r + k, b, right - r, b);
            pb.line_to(l + r, b);
            pb.cubic_to(l + r - k, b, l, b - r + k, l, b - r);
            pb.line_to(l, t + r);
            pb.cubic_to(l, t + r - k, l + r - k, t, l + r, t);
            pb.close();
        }
        if let Some(path) = pb.finish() {
            let mut paint = Paint::default();
            paint.set_color(color(fill));
            paint.anti_alias = true;
            self.pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
        }
    }

    /// A thick circular arc, drawn as a filled ring segment so the ends stay
    /// square against the track underneath.
    fn arc(&mut self, cx: f32, cy: f32, radius: f32, width: f32, from: f32, to: f32, fill: &str) {
        let s = self.scale;
        let (cx, cy, radius, width) = (cx * s, cy * s, radius * s, width * s);
        let (outer, inner) = (radius, radius - width);
        // One segment per degree is well under a pixel of error at this size.
        let steps = ((to - from).abs().ceil() as usize).max(2);
        let mut pb = PathBuilder::new();
        for i in 0..=steps {
            let a = (from + (to - from) * i as f32 / steps as f32).to_radians();
            let (x, y) = (cx + outer * a.cos(), cy + outer * a.sin());
            if i == 0 { pb.move_to(x, y) } else { pb.line_to(x, y) }
        }
        for i in (0..=steps).rev() {
            let a = (from + (to - from) * i as f32 / steps as f32).to_radians();
            pb.line_to(cx + inner * a.cos(), cy + inner * a.sin());
        }
        pb.close();
        if let Some(path) = pb.finish() {
            let mut paint = Paint::default();
            paint.set_color(color(fill));
            paint.anti_alias = true;
            self.pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
        }
    }

    fn text_centered(&mut self, text: &str, y: f32, size: f32, fill: &str, bold: bool) {
        self.text_at(text, GRID / 2.0, y, size, fill, bold);
    }

    /// Draw `text` horizontally centred on `cx`, with `y` as the top of the line.
    fn text_at(&mut self, text: &str, cx: f32, y: f32, size: f32, fill: &str, bold: bool) {
        let w = self.text_width(text, size, bold);
        self.blit_text(text, cx * self.scale - w / 2.0, y, size, fill, bold);
    }

    fn text_left(&mut self, text: &str, x: f32, y: f32, size: f32, fill: &str) {
        self.blit_text(text, x * self.scale, y, size, fill, false);
    }

    fn text_right(&mut self, text: &str, x: f32, y: f32, size: f32, fill: &str) {
        let w = self.text_width(text, size, false);
        self.blit_text(text, x * self.scale - w, y, size, fill, false);
    }

    fn font(&self, bold: bool) -> &FontVec {
        if bold { &self.bold } else { &self.regular }
    }

    /// Advance width in device pixels.
    fn text_width(&self, text: &str, size: f32, bold: bool) -> f32 {
        let font = self.font(bold);
        let scaled = font.as_scaled(PxScale::from(size * self.scale));
        let mut w = 0.0;
        let mut prev: Option<ab_glyph::GlyphId> = None;
        for c in text.chars() {
            let id = scaled.glyph_id(c);
            if let Some(p) = prev {
                w += scaled.kern(p, id);
            }
            w += scaled.h_advance(id);
            prev = Some(id);
        }
        w
    }

    /// Rasterise a string and alpha-blend it into the frame. `x` is the left
    /// edge and `y` the top of the line, both already in device pixels for `x`
    /// and in grid units for `y`.
    fn blit_text(&mut self, text: &str, x: f32, y: f32, size: f32, fill: &str, bold: bool) {
        let px = PxScale::from(size * self.scale);
        let col = color(fill);
        let (cr, cg, cb) = (col.red(), col.green(), col.blue());
        let width = self.size as i32;
        let height = self.size as i32;

        // Position from the top of the line rather than the baseline, matching
        // the way the original layout was measured.
        let font = if bold { &self.bold } else { &self.regular };
        let scaled = font.as_scaled(px);
        let baseline = y * self.scale + scaled.ascent();

        let mut pen = x;
        let mut prev: Option<ab_glyph::GlyphId> = None;
        // Collect first so the immutable font borrow ends before we touch the
        // pixmap buffer mutably.
        let mut outlines = Vec::new();
        for c in text.chars() {
            let id = scaled.glyph_id(c);
            if let Some(p) = prev {
                pen += scaled.kern(p, id);
            }
            let glyph = id.with_scale_and_position(px, ab_glyph::point(pen, baseline));
            if let Some(o) = font.outline_glyph(glyph) {
                outlines.push(o);
            }
            pen += scaled.h_advance(id);
            prev = Some(id);
        }

        let data = self.pixmap.data_mut();
        for outline in outlines {
            let bounds = outline.px_bounds();
            let (ox, oy) = (bounds.min.x as i32, bounds.min.y as i32);
            outline.draw(|gx, gy, coverage| {
                let px_x = ox + gx as i32;
                let px_y = oy + gy as i32;
                if px_x < 0 || px_y < 0 || px_x >= width || px_y >= height {
                    return;
                }
                let a = coverage.clamp(0.0, 1.0);
                if a <= 0.0 {
                    return;
                }
                let i = ((px_y * width + px_x) * 4) as usize;
                // tiny-skia stores premultiplied RGBA, and the source is opaque,
                // so premultiplying is just a scale by coverage.
                let blend = |dst: u8, src: f32| -> u8 {
                    (src * 255.0 * a + dst as f32 * (1.0 - a)).round().clamp(0.0, 255.0) as u8
                };
                data[i] = blend(data[i], cr);
                data[i + 1] = blend(data[i + 1], cg);
                data[i + 2] = blend(data[i + 2], cb);
                data[i + 3] = blend(data[i + 3], 1.0);
            });
        }
    }

    /// Interpolate the readout colour toward the accent as the temperature
    /// climbs, when `color_by_temp` is on.
    fn readout_color(&self, f: &Frame, temp: Option<f32>, accent: &str) -> String {
        if !f.opts.color_by_temp {
            return f.colors.text.clone();
        }
        let Some(t) = temp else { return f.colors.text.clone() };
        let k = fraction(t, f.opts);
        let (a, b) = (color(&f.colors.text), color(accent));
        let mix = |x: f32, y: f32| ((x + (y - x) * k) * 255.0).round() as u8;
        format!(
            "#{:02X}{:02X}{:02X}",
            mix(a.red(), b.red()),
            mix(a.green(), b.green()),
            mix(a.blue(), b.blue())
        )
    }

    /// Rotate the finished frame if the panel is mounted turned. Done here so
    /// the device itself can stay at orientation 0.
    fn apply_rotation(&mut self, degrees: u32) -> &Pixmap {
        let d = degrees % 360;
        if d == 0 {
            return &self.pixmap;
        }
        let size = self.size;
        if self.rotated.is_none() {
            self.rotated = Pixmap::new(size, size);
        }
        let Some(dst) = self.rotated.as_mut() else { return &self.pixmap };

        if d == 180 {
            // A half turn is a straight reversal of the pixel order.
            let src = self.pixmap.data();
            let out = dst.data_mut();
            let n = (size * size) as usize;
            for i in 0..n {
                let j = n - 1 - i;
                out[i * 4..i * 4 + 4].copy_from_slice(&src[j * 4..j * 4 + 4]);
            }
        } else {
            let src = self.pixmap.data();
            let out = dst.data_mut();
            let w = size as usize;
            for y in 0..w {
                for x in 0..w {
                    // Counter-clockwise for 90, matching the rotation direction
                    // of the original Python version (PIL's Image.rotate).
                    let (sx, sy) = if d == 90 { (w - 1 - y, x) } else { (y, w - 1 - x) };
                    let si = (sy * w + sx) * 4;
                    let di = (y * w + x) * 4;
                    out[di..di + 4].copy_from_slice(&src[si..si + 4]);
                }
            }
        }
        self.rotated.as_ref().unwrap()
    }
}

/// Which component, if any, is over its alert threshold. GPU wins ties, as in
/// the original.
fn overheating(f: &Frame) -> Option<&'static str> {
    if f.opts.show_gpu {
        if let Some(g) = f.gpu {
            if g >= f.opts.gpu_alert_temp {
                return Some("GPU");
            }
        }
    }
    if f.opts.show_cpu {
        if let Some(c) = f.cpu {
            if c >= f.opts.cpu_alert_temp {
                return Some("CPU");
            }
        }
    }
    None
}

/// Where a temperature sits in the configured bar range, clamped to 0..1.
fn fraction(temp: f32, o: &Options) -> f32 {
    let span = o.bar_max - o.bar_min;
    if span <= 0.0 {
        return 0.0;
    }
    ((temp.clamp(o.bar_min, o.bar_max) - o.bar_min) / span).clamp(0.0, 1.0)
}

fn temp_text(temp: Option<f32>, o: &Options) -> String {
    match temp {
        Some(t) if o.show_degree_symbol => format!("{}\u{00B0}", t.round() as i32),
        Some(t) => format!("{}", t.round() as i32),
        None => "N/A".into(),
    }
}

/// Encode a frame as a PNG, for the on-disk preview.
pub fn write_png(pixmap: &Pixmap, path: &std::path::Path) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    pixmap.save_png(path).map_err(|e| format!("could not write {}: {e}", path.display()))
}
