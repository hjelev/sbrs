use std::{
    io::{self, Cursor, Write},
    path::PathBuf,
};

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use crossterm::{
    cursor::MoveTo,
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    style::{Color as CtColor, Print, ResetColor, SetBackgroundColor, SetForegroundColor},
    terminal::{disable_raw_mode, enable_raw_mode, Clear as TermClear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use image::{imageops::FilterType, GenericImageView, ImageFormat, ImageReader};
use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
};

use crate::{integration::probe::TerminalImageProtocol, App};

/// Box-filter average of one halfblock pixel cell.
/// `cell_x` and `cell_y` are in scaled-render-pixel coordinates
/// (render_w × render_h).  Returns the averaged Color::Rgb.
fn avg_pixel(
    rgb: &[u8],
    img_w: u32,
    img_h: u32,
    render_w: u32,
    render_h: u32,
    cell_x: u32,
    cell_y: u32,
) -> Color {
    // Map cell rectangle -> source rectangle (fixed-point, exclusive end)
    let src_x0 = (cell_x * img_w / render_w).min(img_w - 1);
    let src_x1 = ((cell_x + 1) * img_w / render_w).min(img_w).max(src_x0 + 1);
    let src_y0 = (cell_y * img_h / render_h).min(img_h - 1);
    let src_y1 = ((cell_y + 1) * img_h / render_h).min(img_h).max(src_y0 + 1);

    let mut r_sum = 0u32;
    let mut g_sum = 0u32;
    let mut b_sum = 0u32;
    let mut count = 0u32;

    for py in src_y0..src_y1 {
        for px in src_x0..src_x1 {
            let idx = ((py * img_w + px) * 3) as usize;
            if idx + 2 < rgb.len() {
                r_sum += rgb[idx] as u32;
                g_sum += rgb[idx + 1] as u32;
                b_sum += rgb[idx + 2] as u32;
                count += 1;
            }
        }
    }

    if count == 0 {
        return Color::Black;
    }
    Color::Rgb(
        (r_sum / count) as u8,
        (g_sum / count) as u8,
        (b_sum / count) as u8,
    )
}

impl App {
    /// Compute a centered, aspect-preserving cell rectangle for native image protocols.
    /// Uses a 1:2 cell aspect model (terminal cells are roughly twice as tall as wide).
    pub(crate) fn fit_native_image_area(area: Rect, img_w: u32, img_h: u32) -> Rect {
        if area.width == 0 || area.height == 0 || img_w == 0 || img_h == 0 {
            return area;
        }

        let avail_w = area.width as f32;
        let avail_h = (area.height as f32) * 2.0;
        let img_ratio = img_w as f32 / img_h as f32;
        let avail_ratio = avail_w / avail_h;

        let (fit_w, fit_h) = if img_ratio >= avail_ratio {
            let w = area.width;
            let h = (((w as f32 / img_ratio) / 2.0).floor() as u16).max(1).min(area.height);
            (w, h)
        } else {
            let h = area.height;
            let w = (((h as f32 * 2.0) * img_ratio).floor() as u16).max(1).min(area.width);
            (w, h)
        };

        let x = area.x + area.width.saturating_sub(fit_w) / 2;
        let y = area.y + area.height.saturating_sub(fit_h) / 2;
        Rect::new(x, y, fit_w, fit_h)
    }

    // -------------------------------------------------------------------------
    // Full-screen halfblock fallback preview
    // -------------------------------------------------------------------------

    /// Full-screen fallback preview that works on standard terminals by drawing
    /// true-color halfblocks (no native image protocol required).
    pub(crate) fn preview_images_with_halfblock_fullscreen(
        &mut self,
        start_path: PathBuf,
    ) -> io::Result<bool> {
        let images: Vec<PathBuf> = self
            .entries
            .iter()
            .map(|e| e.path())
            .filter(Self::is_image_file)
            .collect();

        if images.is_empty() {
            return Ok(false);
        }

        let start_idx = images.iter().position(|p| *p == start_path).unwrap_or(0);

        disable_raw_mode()?;
        execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen)?;

        let fallback_result = Self::render_halfblock_fullscreen_slideshow(&images, start_idx);

        execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
        execute!(io::stdout(), TermClear(ClearType::All), MoveTo(0, 0))?;
        enable_raw_mode()?;
        Self::drain_pending_terminal_events();

        match fallback_result {
            Ok(idx) => {
                let selected = images[idx].clone();
                if let Some(name) = selected.file_name() {
                    self.select_entry_named(&name.to_string_lossy());
                }
                Ok(true)
            }
            Err(err_msg) => {
                self.set_status(format!("image preview failed: {}", err_msg));
                Ok(false)
            }
        }
    }

    fn render_halfblock_fullscreen_slideshow(
        images: &[PathBuf],
        start_idx: usize,
    ) -> Result<usize, String> {
        if images.is_empty() {
            return Err("no images to preview".to_string());
        }

        enable_raw_mode().map_err(|e| e.to_string())?;
        let run_result = (|| -> Result<usize, String> {
            let mut idx = start_idx.min(images.len() - 1);

            loop {
                execute!(io::stdout(), TermClear(ClearType::All), MoveTo(0, 0))
                    .map_err(|e| e.to_string())?;

                let (tw, th) = crossterm::terminal::size().map_err(|e| e.to_string())?;
                let image_rows = th.saturating_sub(1).max(1);

                if let Some((rgb, iw, ih)) = Self::decode_image_to_rgb_scaled(&images[idx]) {
                    Self::draw_halfblock_terminal(&rgb, iw, ih, tw, image_rows)
                        .map_err(|e| e.to_string())?;
                } else {
                    execute!(
                        io::stdout(),
                        MoveTo(0, 0),
                        ResetColor,
                        Print("[image could not be decoded]")
                    )
                    .map_err(|e| e.to_string())?;
                }

                let help_row = th.saturating_sub(1);
                execute!(
                    io::stdout(),
                    MoveTo(0, help_row),
                    ResetColor,
                    Print(format!(
                        "[halfblock] {}/{}  [←/→ prev/next (exits at ends), q/Esc/Enter exit]",
                        idx + 1,
                        images.len()
                    ))
                )
                .map_err(|e| e.to_string())?;
                io::stdout().flush().map_err(|e| e.to_string())?;

                let key = loop {
                    if event::poll(std::time::Duration::from_millis(120)).map_err(|e| e.to_string())? {
                        if let Event::Key(k) = event::read().map_err(|e| e.to_string())? {
                            break k;
                        }
                    }
                };

                match key.code {
                    KeyCode::Left => {
                        if idx == 0 {
                            return Ok(idx);
                        }
                        idx -= 1;
                    }
                    KeyCode::Right => {
                        if idx + 1 >= images.len() {
                            return Ok(idx);
                        }
                        idx += 1;
                    }
                    KeyCode::Char('q') | KeyCode::Esc | KeyCode::Enter => return Ok(idx),
                    _ => {}
                }
            }
        })();

        let _ = disable_raw_mode();
        run_result
    }

    fn draw_halfblock_terminal(
        rgb_data: &[u8],
        img_w: u32,
        img_h: u32,
        display_w: u16,
        display_h: u16,
    ) -> io::Result<()> {
        if display_w == 0 || display_h == 0 || img_w == 0 || img_h == 0 || rgb_data.is_empty() {
            return Ok(());
        }

        let dw = display_w as u32;
        let dh = display_h as u32;
        let pixel_h = dh * 2;
        let scale = (dw as f32 / img_w as f32).min(pixel_h as f32 / img_h as f32);
        let render_w = ((img_w as f32 * scale) as u32).max(1).min(dw);
        let render_h = ((img_h as f32 * scale) as u32).max(1).min(pixel_h);
        let x_offset = (dw - render_w) / 2;
        let y_offset = (pixel_h - render_h) / 2;

        let mut out = io::stdout();
        for row in 0..dh {
            execute!(out, MoveTo(0, row as u16))?;
            let py_top = row * 2;
            let py_bot = row * 2 + 1;
            let top_in = py_top >= y_offset && py_top < y_offset + render_h;
            let bot_in = py_bot >= y_offset && py_bot < y_offset + render_h;

            for col in 0..dw {
                if !top_in && !bot_in || col < x_offset || col >= x_offset + render_w {
                    execute!(out, ResetColor, Print(" "))?;
                    continue;
                }

                let cell_x = col - x_offset;
                let fg = if top_in {
                    avg_pixel(rgb_data, img_w, img_h, render_w, render_h, cell_x, py_top - y_offset)
                } else {
                    Color::Black
                };
                let bg = if bot_in {
                    avg_pixel(rgb_data, img_w, img_h, render_w, render_h, cell_x, py_bot - y_offset)
                } else {
                    Color::Black
                };

                let fg = match fg {
                    Color::Rgb(r, g, b) => CtColor::Rgb { r, g, b },
                    _ => CtColor::Black,
                };
                let bg = match bg {
                    Color::Rgb(r, g, b) => CtColor::Rgb { r, g, b },
                    _ => CtColor::Black,
                };

                execute!(
                    out,
                    SetForegroundColor(fg),
                    SetBackgroundColor(bg),
                    Print("▀")
                )?;
            }
            execute!(out, ResetColor)?;
        }

        out.flush()
    }

    // -------------------------------------------------------------------------
    // Decode helpers (called from preview worker thread)
    // -------------------------------------------------------------------------

    /// Decode an image file to raw RGB bytes, scaled to fit within 400×200 px.
    /// Returns `(rgb_bytes, width, height)` or `None` on decode failure.
    pub(crate) fn decode_image_to_rgb_scaled(path: &PathBuf) -> Option<(Vec<u8>, u32, u32)> {
        let reader = ImageReader::open(path).ok()?;
        let img = reader.decode().ok()?;
        let (w, h) = img.dimensions();

        // Cap at 2048×2048 only to bound memory usage; halfblock_lines handles
        // the final downsample to terminal cell dimensions at render time.
        let max_w = 2048u32;
        let max_h = 2048u32;
        let (new_w, new_h) = if w <= max_w && h <= max_h {
            (w, h)
        } else {
            let scale = (max_w as f32 / w.max(1) as f32)
                .min(max_h as f32 / h.max(1) as f32);
            (((w as f32 * scale) as u32).max(1), ((h as f32 * scale) as u32).max(1))
        };

        let resized = if new_w == w && new_h == h {
            img.to_rgb8()
        } else {
            img.resize(new_w, new_h, FilterType::Lanczos3).to_rgb8()
        };
        let (rw, rh) = (resized.width(), resized.height());
        Some((resized.into_raw(), rw, rh))
    }

    // -------------------------------------------------------------------------
    // Halfblock pane renderer (ratatui-native, works on all true-color terminals)
    // -------------------------------------------------------------------------

    /// Convert an RGB image buffer to ratatui `Line`s using "▀" half-block chars.
    /// Each character cell covers 2 pixel rows: top pixel = foreground color,
    /// bottom pixel = background color.  Output is `display_h` lines of `display_w`
    /// spans — fits directly inside a ratatui `Paragraph`.
    pub(crate) fn halfblock_lines(
        rgb_data: &[u8],
        img_w: u32,
        img_h: u32,
        display_w: u16,
        display_h: u16,
    ) -> Vec<Line<'static>> {
        if display_w == 0 || display_h == 0 || img_w == 0 || img_h == 0 || rgb_data.is_empty() {
            return Vec::new();
        }

        let dw = display_w as u32;
        let dh = display_h as u32;

        // Keep aspect ratio: compute the source region that fits inside display_w × (display_h*2)
        // without stretching.
        let pixel_h = dh * 2; // pixel rows we have to fill
        let scale_w = dw as f32 / img_w as f32;
        let scale_h = pixel_h as f32 / img_h as f32;
        let scale = scale_w.min(scale_h);
        let render_w = ((img_w as f32 * scale) as u32).max(1).min(dw);
        let render_h = ((img_h as f32 * scale) as u32).max(1).min(pixel_h);

        let x_offset = (dw - render_w) / 2;
        let y_offset = (pixel_h - render_h) / 2; // vertical centering offset (in pixel rows)

        let mut lines: Vec<Line<'static>> = Vec::with_capacity(dh as usize);

        for row in 0..dh {
            let py_top = row * 2;
            let py_bot = row * 2 + 1;

            // Check if both pixel rows are outside the rendered image region
            let top_in = py_top >= y_offset && py_top < y_offset + render_h;
            let bot_in = py_bot >= y_offset && py_bot < y_offset + render_h;

            if !top_in && !bot_in {
                // Entire row is padding — emit blank line
                lines.push(Line::from(vec![Span::raw(" ".repeat(dw as usize))]));
                continue;
            }

            let mut spans: Vec<Span<'static>> = Vec::with_capacity(dw as usize);

            for col in 0..dw {
                if col < x_offset || col >= x_offset + render_w {
                    spans.push(Span::raw(" "));
                    continue;
                }

                let cell_x = col - x_offset;

                let fg = if top_in {
                    avg_pixel(rgb_data, img_w, img_h, render_w, render_h, cell_x, py_top - y_offset)
                } else {
                    Color::Black
                };
                let bg = if bot_in {
                    avg_pixel(rgb_data, img_w, img_h, render_w, render_h, cell_x, py_bot - y_offset)
                } else {
                    Color::Black
                };

                spans.push(Span::styled("▀", Style::default().fg(fg).bg(bg)));
            }

            lines.push(Line::from(spans));
        }

        lines
    }

    // -------------------------------------------------------------------------
    // In-pane Kitty GP rendering helpers
    // -------------------------------------------------------------------------

    /// Encode raw RGB bytes to a PNG byte buffer (used for Kitty/iTerm2 pane emission).
    pub(crate) fn encode_rgb_to_png(rgb: &[u8], w: u32, h: u32) -> Option<Vec<u8>> {
        use image::{ImageFormat, RgbImage};
        let img = RgbImage::from_raw(w, h, rgb.to_vec())?;
        let dyn_img = image::DynamicImage::ImageRgb8(img);
        let mut buf = std::io::Cursor::new(Vec::new());
        dyn_img.write_to(&mut buf, ImageFormat::Png).ok()?;
        Some(buf.into_inner())
    }

    /// Emit a Kitty Graphics Protocol image directly into the preview pane area.
    /// Positions the cursor at (`pane_x`, `pane_y`) and transmits the image sized
    /// to `cols` × `rows` terminal cells.
    pub(crate) fn clear_kitty_pane_images() -> io::Result<()> {
        let mut out = io::stdout();
        // Remove all visible Kitty image placements before drawing the next preview.
        // This prevents old frames from lingering when aspect-fit bounds change.
        write!(out, "\x1b_Ga=d,d=A\x1b\\")?;
        out.flush()
    }

    pub(crate) fn emit_kitty_pane(
        png_data: &[u8],
        img_w: u32,
        img_h: u32,
        pane_x: u16,
        pane_y: u16,
        cols: u16,
        rows: u16,
    ) -> io::Result<()> {
        use crossterm::cursor::MoveTo;
        use crossterm::execute;
        let payload = BASE64_STANDARD.encode(png_data);
        let chunk_size = 4096;
        let bytes = payload.as_bytes();
        let mut out = io::stdout();
        execute!(out, MoveTo(pane_x, pane_y))?;
        let mut offset = 0;
        while offset < bytes.len() {
            let end = (offset + chunk_size).min(bytes.len());
            let chunk = &bytes[offset..end];
            let more = if end < bytes.len() { 1 } else { 0 };
            if offset == 0 {
                write!(
                    out,
                    "\x1b_Ga=T,f=100,s={},v={},c={},r={},m={};{}\x1b\\",
                    img_w, img_h, cols, rows, more,
                    std::str::from_utf8(chunk).unwrap_or("")
                )?;
            } else {
                write!(out, "\x1b_Gm={};{}\x1b\\", more,
                    std::str::from_utf8(chunk).unwrap_or(""))?;
            }
            offset = end;
        }
        out.flush()
    }

    // -------------------------------------------------------------------------
    // Full-screen native protocol preview (called on Enter key)
    // -------------------------------------------------------------------------

    /// Attempt native-protocol full-screen image preview. Leaves alternate screen,
    /// renders the image, waits for Enter, then returns to TUI.
    /// Returns `Ok(true)` when the image was shown; `Ok(false)` when the protocol
    /// is unsupported or the integration is disabled (caller falls back to viu/chafa).
    pub(crate) fn preview_images_with_native(&mut self, start_path: PathBuf) -> io::Result<bool> {
        let (protocol, _) = Self::terminal_image_protocol();
        if protocol == TerminalImageProtocol::Unsupported {
            return Ok(false);
        }
        if !self.integration_active("image-native") {
            return Ok(false);
        }

        let images: Vec<PathBuf> = self
            .entries
            .iter()
            .map(|e| e.path())
            .filter(Self::is_image_file)
            .collect();

        if images.is_empty() {
            return Ok(false);
        }

        let start_idx = images.iter().position(|p| *p == start_path).unwrap_or(0);

        disable_raw_mode()?;
        execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen)?;

        let native_result = Self::render_native_fullscreen_slideshow(&images, start_idx, protocol);

        execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
        execute!(io::stdout(), TermClear(ClearType::All), MoveTo(0, 0))?;
        enable_raw_mode()?;
        Self::drain_pending_terminal_events();

        match native_result {
            Ok(idx) => {
                let selected = images[idx].clone();
                if let Some(name) = selected.file_name() {
                    self.select_entry_named(&name.to_string_lossy());
                }
                Ok(true)
            }
            Err(err_msg) => {
                self.set_status(format!("native image preview failed: {}", err_msg));
                Ok(false)
            }
        }
    }

    fn render_native_fullscreen_slideshow(
        images: &[PathBuf],
        start_idx: usize,
        protocol: TerminalImageProtocol,
    ) -> Result<usize, String> {
        if images.is_empty() {
            return Err("no images to preview".to_string());
        }

        enable_raw_mode().map_err(|e| e.to_string())?;
        let run_result = (|| -> Result<usize, String> {
            let mut idx = start_idx.min(images.len() - 1);

            loop {
                execute!(io::stdout(), TermClear(ClearType::All), MoveTo(0, 0))
                    .map_err(|e| e.to_string())?;
                Self::emit_native_image_protocol(&images[idx], protocol, 2400)?;

                println!(
                    "\n  [{}]  {}/{}  [←/→ prev/next (exits at ends), q/Esc/Enter exit]",
                    protocol.label(),
                    idx + 1,
                    images.len()
                );
                io::stdout().flush().map_err(|e| e.to_string())?;

                let key = loop {
                    if event::poll(std::time::Duration::from_millis(120)).map_err(|e| e.to_string())? {
                        if let Event::Key(k) = event::read().map_err(|e| e.to_string())? {
                            break k;
                        }
                    }
                };

                match key.code {
                    KeyCode::Left => {
                        if idx == 0 {
                            return Ok(idx);
                        }
                        idx -= 1;
                    }
                    KeyCode::Right => {
                        if idx + 1 >= images.len() {
                            return Ok(idx);
                        }
                        idx += 1;
                    }
                    KeyCode::Char('q') | KeyCode::Esc | KeyCode::Enter => {
                        return Ok(idx);
                    }
                    _ => {}
                }
            }
        })();

        let _ = disable_raw_mode();
        run_result
    }

    /// Decode the image, scale it, and emit the protocol payload to stdout.
    pub(crate) fn emit_native_image_protocol(
        path: &PathBuf,
        protocol: TerminalImageProtocol,
        max_edge: u32,
    ) -> Result<(), String> {
        match protocol {
            TerminalImageProtocol::Kitty => Self::emit_kitty(path, max_edge),
            TerminalImageProtocol::Iterm2Inline => Self::emit_iterm2(path, max_edge),
            TerminalImageProtocol::Sixel => Self::emit_sixel(path, max_edge),
            TerminalImageProtocol::Unsupported => Err("unsupported protocol".to_string()),
        }
    }

    fn decode_to_png_bytes(path: &PathBuf, max_edge: u32) -> Result<(Vec<u8>, u32, u32), String> {
        let reader = ImageReader::open(path).map_err(|e| e.to_string())?;
        let mut img = reader.decode().map_err(|e| e.to_string())?;
        let (w, h) = img.dimensions();
        if w > max_edge || h > max_edge {
            img = img.resize(max_edge, max_edge, FilterType::Triangle);
        }
        let (sw, sh) = img.dimensions();
        let mut buf = Cursor::new(Vec::new());
        img.write_to(&mut buf, ImageFormat::Png).map_err(|e| e.to_string())?;
        Ok((buf.into_inner(), sw, sh))
    }

    fn decode_to_rgb_bytes(path: &PathBuf, max_edge: u32) -> Result<(Vec<u8>, u32, u32), String> {
        let reader = ImageReader::open(path).map_err(|e| e.to_string())?;
        let mut img = reader.decode().map_err(|e| e.to_string())?;
        let (w, h) = img.dimensions();
        if w > max_edge || h > max_edge {
            img = img.resize(max_edge, max_edge, FilterType::Triangle);
        }
        let rgb = img.to_rgb8();
        let (sw, sh) = (rgb.width(), rgb.height());
        Ok((rgb.into_raw(), sw, sh))
    }

    fn emit_kitty(path: &PathBuf, max_edge: u32) -> Result<(), String> {
        let (png_data, w, h) = Self::decode_to_png_bytes(path, max_edge)?;
        let payload = BASE64_STANDARD.encode(&png_data);

        // Kitty graphics protocol: chunked transmission, f=100 (PNG), t=d (direct)
        let chunk_size = 4096;
        let bytes = payload.as_bytes();
        let mut out = io::stdout();
        let mut offset = 0;

        while offset < bytes.len() {
            let end = (offset + chunk_size).min(bytes.len());
            let chunk = &bytes[offset..end];
            let more = if end < bytes.len() { 1 } else { 0 };
            if offset == 0 {
                write!(out, "\x1b_Ga=T,f=100,s={},v={},m={};{}\x1b\\", w, h, more,
                    std::str::from_utf8(chunk).unwrap_or("")).map_err(|e| e.to_string())?;
            } else {
                write!(out, "\x1b_Gm={};{}\x1b\\", more,
                    std::str::from_utf8(chunk).unwrap_or("")).map_err(|e| e.to_string())?;
            }
            offset = end;
        }
        out.flush().map_err(|e| e.to_string())
    }

    fn emit_iterm2(path: &PathBuf, max_edge: u32) -> Result<(), String> {
        let (png_data, _, _) = Self::decode_to_png_bytes(path, max_edge)?;
        let payload = BASE64_STANDARD.encode(&png_data);
        let mut out = io::stdout();
        write!(
            out,
            "\x1b]1337;File=inline=1;preserveAspectRatio=1;width=auto;height=auto:{}\x07",
            payload
        )
        .map_err(|e| e.to_string())?;
        out.flush().map_err(|e| e.to_string())
    }

    fn emit_sixel(path: &PathBuf, max_edge: u32) -> Result<(), String> {
        let (rgb, w, h) = Self::decode_to_rgb_bytes(path, max_edge)?;

        let mut rgba: Vec<u8> = Vec::with_capacity((w as usize) * (h as usize) * 4);
        for chunk in rgb.chunks_exact(3) {
            rgba.push(chunk[0]);
            rgba.push(chunk[1]);
            rgba.push(chunk[2]);
            rgba.push(255);
        }

        let image = icy_sixel::SixelImage::try_from_rgba(rgba, w as usize, h as usize)
            .map_err(|e| format!("sixel image error: {}", e))?;
        let sixel_out = image
            .encode()
            .map_err(|e| format!("sixel encode error: {}", e))?;

        let mut out = io::stdout();
        out.write_all(sixel_out.as_bytes()).map_err(|e| e.to_string())?;
        out.flush().map_err(|e| e.to_string())
    }
}
