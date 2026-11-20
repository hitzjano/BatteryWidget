//! Draws the widget with Direct2D/DirectWrite into a 32-bit bitmap and hands it to the
//! layered window. The window therefore has true per-pixel transparency, and nothing is
//! drawn unless the content actually changed.

use std::ffi::c_void;

use battery_widget_core::{Device, Kind};
use windows::core::{w, Result, BOOL, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, POINT, RECT, SIZE};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_END_CLOSED,
    D2D1_PIXEL_FORMAT, D2D_RECT_F,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1DCRenderTarget, ID2D1Factory, ID2D1SolidColorBrush,
    D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_FEATURE_LEVEL_DEFAULT,
    D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_SOFTWARE, D2D1_RENDER_TARGET_USAGE_NONE,
    D2D1_ROUNDED_RECT, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat, DWRITE_FACTORY_TYPE_SHARED,
    DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_NORMAL,
    DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_MEASURING_MODE_NATURAL, DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
    DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_METRICS, DWRITE_WORD_WRAPPING_NO_WRAP,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject,
    AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS,
    HBITMAP, HDC, HGDIOBJ,
};
use windows::Win32::UI::WindowsAndMessaging::{UpdateLayeredWindow, ULW_ALPHA};
use windows_numerics::Vector2;

use crate::layout::{self, Display, Layout, Pill};
use crate::theme::{self, Palette, Rgba};

const TEXT_FONT: PCWSTR = w!("Segoe UI Variable Text");
const TEXT_FONT_FALLBACK: PCWSTR = w!("Segoe UI");
/// Windows 11 ships "Segoe Fluent Icons", Windows 10 "Segoe MDL2 Assets"; the code points
/// used here exist in both.
const ICON_FONTS: [PCWSTR; 2] = [w!("Segoe Fluent Icons"), w!("Segoe MDL2 Assets")];

fn glyph(kind: Kind) -> char {
    match kind {
        Kind::Keyboard => '\u{E765}',
        Kind::Mouse => '\u{E962}',
        Kind::Headset => '\u{E7F6}',
        Kind::Gamepad => '\u{E7FC}',
        Kind::Speaker => '\u{E7F5}',
        Kind::Phone => '\u{E8EA}',
        Kind::Tablet => '\u{E70A}',
        Kind::Watch => '\u{E916}',
        Kind::Computer => '\u{E7F8}',
        Kind::Pen => '\u{E76D}',
        Kind::Microphone => '\u{E720}',
        Kind::Camera => '\u{E722}',
        Kind::Tv => '\u{E7F4}',
        Kind::Remote | Kind::Generic => '\u{E772}',
    }
}

fn color(c: Rgba) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: c[0],
        g: c[1],
        b: c[2],
        a: c[3],
    }
}

fn rect(x: f32, y: f32, width: f32, height: f32) -> D2D_RECT_F {
    D2D_RECT_F {
        left: x,
        top: y,
        right: x + width,
        bottom: y + height,
    }
}

fn rounded(r: D2D_RECT_F, radius: f32) -> D2D1_ROUNDED_RECT {
    D2D1_ROUNDED_RECT {
        rect: r,
        radiusX: radius,
        radiusY: radius,
    }
}

/// GDI surface the widget is drawn into; released on drop.
struct Surface {
    screen: HDC,
    memory: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
}

impl Surface {
    fn new(width: i32, height: i32) -> Result<Surface> {
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                // Negative height: top-down rows, the layout Direct2D expects.
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        // SAFETY: standard GDI set-up; every object is released in `drop`.
        unsafe {
            let screen = GetDC(None);
            let memory = CreateCompatibleDC(Some(screen));
            let mut bits: *mut c_void = std::ptr::null_mut();
            let bitmap =
                match CreateDIBSection(Some(screen), &info, DIB_RGB_COLORS, &mut bits, None, 0) {
                    Ok(bitmap) => bitmap,
                    Err(error) => {
                        let _ = DeleteDC(memory);
                        ReleaseDC(None, screen);
                        return Err(error);
                    }
                };
            let previous = SelectObject(memory, bitmap.into());
            Ok(Surface {
                screen,
                memory,
                bitmap,
                previous,
            })
        }
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        // SAFETY: the handles were created in `new` and are released exactly once.
        unsafe {
            SelectObject(self.memory, self.previous);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.memory);
            ReleaseDC(None, self.screen);
        }
    }
}

pub struct Renderer {
    d2d: ID2D1Factory,
    dwrite: IDWriteFactory,
    text_font: PCWSTR,
    icon_font: Option<PCWSTR>,
}

impl Renderer {
    pub fn new() -> Result<Renderer> {
        // SAFETY: factory creation with valid arguments.
        let (d2d, dwrite) = unsafe {
            (
                D2D1CreateFactory::<ID2D1Factory>(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?,
                DWriteCreateFactory::<IDWriteFactory>(DWRITE_FACTORY_TYPE_SHARED)?,
            )
        };
        let installed = |family: PCWSTR| -> bool {
            let mut collection = None;
            let (mut index, mut exists) = (0u32, BOOL(0));
            // SAFETY: out-pointers reference live locals.
            unsafe {
                dwrite
                    .GetSystemFontCollection(&mut collection, false)
                    .is_ok()
                    && collection
                        .is_some_and(|c| c.FindFamilyName(family, &mut index, &mut exists).is_ok())
                    && exists.as_bool()
            }
        };
        let text_font = if installed(TEXT_FONT) {
            TEXT_FONT
        } else {
            TEXT_FONT_FALLBACK
        };
        let icon_font = ICON_FONTS.into_iter().find(|font| installed(*font));
        Ok(Renderer {
            d2d,
            dwrite,
            text_font,
            icon_font,
        })
    }

    fn format(
        &self,
        family: PCWSTR,
        size: f32,
        bold: bool,
        centered: bool,
    ) -> Result<IDWriteTextFormat> {
        let weight = if bold {
            DWRITE_FONT_WEIGHT_SEMI_BOLD
        } else {
            DWRITE_FONT_WEIGHT_NORMAL
        };
        // SAFETY: all strings are static, NUL-terminated wide strings.
        unsafe {
            let format = self.dwrite.CreateTextFormat(
                family,
                None,
                weight,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                size,
                w!(""),
            )?;
            format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            if centered {
                format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            }
            Ok(format)
        }
    }

    /// The format used for names and percentages at this DPI scale.
    pub fn label_format(&self, scale: f32) -> Result<IDWriteTextFormat> {
        self.format(self.text_font, layout::TEXT_SIZE * scale, true, false)
    }

    /// Width of `text` in pixels.
    pub fn text_width(&self, format: &IDWriteTextFormat, text: &str) -> f32 {
        let wide: Vec<u16> = text.encode_utf16().collect();
        let mut metrics = DWRITE_TEXT_METRICS::default();
        // SAFETY: `metrics` is a valid out-pointer.
        let measured = unsafe {
            self.dwrite
                .CreateTextLayout(&wide, format, 4096.0, 256.0)
                .and_then(|text_layout| text_layout.GetMetrics(&mut metrics))
        };
        match measured {
            Ok(()) => metrics.widthIncludingTrailingWhitespace,
            // A rough estimate keeps the layout usable if DirectWrite fails.
            Err(_) => text.chars().count() as f32 * 7.0,
        }
    }

    /// Draw `layout` and show it in `hwnd`, moving its top-left corner to `origin` (screen
    /// coordinates) if given. A child window is moved by the caller instead.
    pub fn present(
        &self,
        hwnd: HWND,
        origin: Option<POINT>,
        layout: &Layout,
        devices: &[Device],
        palette: &Palette,
        low_threshold: u8,
    ) -> Result<()> {
        let (width, height) = (layout.width.ceil() as i32, layout.height.ceil() as i32);
        if width <= 0 || height <= 0 {
            return Ok(());
        }
        let surface = Surface::new(width, height)?;
        let properties = D2D1_RENDER_TARGET_PROPERTIES {
            // Software rendering: the surface is tiny and this keeps the process free of
            // GPU resources (and of device-lost handling).
            r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            usage: D2D1_RENDER_TARGET_USAGE_NONE,
            minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
        };
        // SAFETY: the render target only lives inside this block, as does the surface it
        // is bound to; all pointers passed reference live locals.
        unsafe {
            let target = self.d2d.CreateDCRenderTarget(&properties)?;
            target.BindDC(
                surface.memory,
                &RECT {
                    left: 0,
                    top: 0,
                    right: width,
                    bottom: height,
                },
            )?;
            target.BeginDraw();
            target.Clear(Some(&D2D1_COLOR_F {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            }));
            // Sub-pixel (ClearType) text cannot be blended onto a transparent surface.
            target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            let brush = target.CreateSolidColorBrush(&color(palette.text), None)?;
            let painter = Painter {
                renderer: self,
                target: &target,
                brush: &brush,
                palette,
                low_threshold,
                scale: layout.scale,
                height: layout.height,
                label: self.label_format(layout.scale)?,
                inner: self.format(
                    self.text_font,
                    layout::INNER_TEXT_SIZE * layout.scale,
                    true,
                    true,
                )?,
                display: layout.display,
                icon: match self.icon_font {
                    Some(font) => {
                        Some(self.format(font, layout::ICON_SIZE * layout.scale, false, true)?)
                    }
                    None => None,
                },
            };
            for pill in &layout.pills {
                painter.pill(pill, pill.device.and_then(|index| devices.get(index)))?;
            }
            target.EndDraw(None, None)?;

            let size = SIZE {
                cx: width,
                cy: height,
            };
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            UpdateLayeredWindow(
                hwnd,
                Some(surface.screen),
                origin.as_ref().map(|o| o as *const POINT),
                Some(&size),
                Some(surface.memory),
                Some(&POINT::default()),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            )
        }
    }
}

struct Painter<'a> {
    renderer: &'a Renderer,
    target: &'a ID2D1DCRenderTarget,
    brush: &'a ID2D1SolidColorBrush,
    palette: &'a Palette,
    low_threshold: u8,
    scale: f32,
    height: f32,
    label: IDWriteTextFormat,
    /// Small, centred text for the percentage inside the battery.
    inner: IDWriteTextFormat,
    icon: Option<IDWriteTextFormat>,
    display: Display,
}

impl Painter<'_> {
    fn s(&self, value: f32) -> f32 {
        value * self.scale
    }

    unsafe fn text(&self, text: &str, format: &IDWriteTextFormat, x: f32, width: f32, c: Rgba) {
        let wide: Vec<u16> = text.encode_utf16().collect();
        unsafe {
            self.brush.SetColor(&color(c));
            self.target.DrawText(
                &wide,
                format,
                &rect(x, 0.0, width, self.height),
                self.brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
    }

    unsafe fn pill(&self, pill: &Pill, device: Option<&Device>) -> Result<()> {
        unsafe {
            let background = rect(pill.x, 0.0, pill.width, self.height);
            self.brush.SetColor(&color(self.palette.pill));
            self.target
                .FillRoundedRectangle(&rounded(background, self.s(layout::RADIUS)), self.brush);
            if self.palette.border[3] > 0.0 {
                let inset = rect(pill.x + 0.5, 0.5, pill.width - 1.0, self.height - 1.0);
                self.brush.SetColor(&color(self.palette.border));
                self.target.DrawRoundedRectangle(
                    &rounded(inset, self.s(layout::RADIUS)),
                    self.brush,
                    1.0,
                    None,
                );
            }

            let mut x = pill.x + self.s(layout::PILL_PADDING);
            let Some(device) = device else {
                // The "+N" overflow pill.
                self.text(
                    &pill.label,
                    &self.label,
                    x,
                    pill.label_width + 1.0,
                    theme::with_alpha(self.palette.text, 0.8),
                );
                return Ok(());
            };
            // Devices that are asleep keep their last level, visibly dimmed.
            let dim = if device.idle || !device.connected {
                0.5
            } else {
                1.0
            };

            if let Some(icon) = &self.icon {
                let glyph = glyph(device.kind).to_string();
                let tint = theme::with_alpha(self.palette.glyph, dim);
                self.text(&glyph, icon, x, self.s(layout::ICON_BOX), tint);
            }
            x += self.s(layout::ICON_BOX + layout::ITEM_GAP);

            if let Some(name) = &pill.name {
                let tint = theme::with_alpha(self.palette.text, 0.85 * dim);
                self.text(name, &self.label, x, pill.name_width + 1.0, tint);
                x += pill.name_width + self.s(layout::ITEM_GAP);
            }

            let text = theme::with_alpha(self.palette.text, dim);
            let small = (
                self.s(layout::BATTERY_WIDTH),
                self.s(layout::BATTERY_HEIGHT),
            );
            match self.display {
                Display::Full => {
                    self.battery(x, small, device, dim, None)?;
                    x += self.s(layout::BATTERY_WIDTH + layout::BATTERY_NUB + layout::ITEM_GAP);
                    self.text(&pill.label, &self.label, x, pill.label_width + 1.0, text);
                }
                Display::IconBar => self.battery(x, small, device, dim, None)?,
                Display::IconPercent => {
                    // Without a bar the colour of the number carries the warning.
                    let low =
                        device.battery.is_some_and(|l| l <= self.low_threshold) && !device.charging;
                    let tint = if low {
                        theme::with_alpha(theme::opaque(theme::LOW), dim)
                    } else {
                        text
                    };
                    self.text(&pill.label, &self.label, x, pill.label_width + 1.0, tint);
                    x += pill.label_width;
                }
                Display::BarPercent => {
                    let big = (
                        self.s(layout::BIG_BATTERY_WIDTH),
                        self.s(layout::BIG_BATTERY_HEIGHT),
                    );
                    self.battery(x, big, device, dim, Some(&pill.label))?;
                    x += self.s(layout::BIG_BATTERY_WIDTH + layout::BATTERY_NUB);
                }
            }
            if device.charging && self.display.bolt_beside() {
                let slot = self.s(layout::BOLT_SLOT);
                self.bolt(
                    x + slot / 2.0 + self.s(1.0),
                    self.height / 2.0,
                    self.s(13.0),
                    dim,
                )?;
            }
            Ok(())
        }
    }

    /// Battery glyph of the given size at `x`. With `inner` the text is written inside
    /// and the charging bolt is left to the caller.
    unsafe fn battery(
        &self,
        x: f32,
        size: (f32, f32),
        device: &Device,
        dim: f32,
        inner: Option<&str>,
    ) -> Result<()> {
        unsafe {
            let (width, height) = size;
            let y = ((self.height - height) / 2.0).round();
            let stroke = self.s(1.0).max(1.0);
            let outline = theme::with_alpha(self.palette.glyph, dim);

            let body = rect(
                x + stroke / 2.0,
                y + stroke / 2.0,
                width - stroke,
                height - stroke,
            );
            self.brush.SetColor(&color(outline));
            let corner = self.s(if inner.is_some() { 3.5 } else { 2.5 });
            self.target
                .DrawRoundedRectangle(&rounded(body, corner), self.brush, stroke, None);
            let nub_height = height * 0.4;
            let nub = rect(
                x + width + self.s(0.5),
                y + (height - nub_height) / 2.0,
                self.s(layout::BATTERY_NUB - 0.5),
                nub_height,
            );
            self.target
                .FillRoundedRectangle(&rounded(nub, self.s(0.75)), self.brush);

            if let Some(level) = device.battery {
                let inset = stroke + self.s(1.0);
                let span = width - inset * 2.0;
                let filled = (span * f32::from(level.min(100)) / 100.0).max(self.s(1.5));
                let fill = theme::level_color(level, device.charging, self.low_threshold);
                // Behind text the fill is toned down so the number stays readable.
                let strength = if inner.is_some() { 0.55 } else { 1.0 };
                self.brush.SetColor(&color(theme::with_alpha(
                    theme::opaque(fill),
                    dim * strength,
                )));
                let bar = rect(x + inset, y + inset, filled, height - inset * 2.0);
                self.target
                    .FillRoundedRectangle(&rounded(bar, corner - self.s(1.5)), self.brush);
            }
            match inner {
                Some(label) => {
                    let tint = theme::with_alpha(self.palette.text, dim);
                    self.text(label, &self.inner, x, width, tint);
                }
                None if device.charging => {
                    self.bolt(x + width / 2.0, y + height / 2.0, height + self.s(4.0), dim)?;
                }
                None => {}
            }
            Ok(())
        }
    }

    /// Lightning bolt centred on `(cx, cy)`. Drawn as geometry so it does not depend on
    /// any font; a thin outline in the background colour keeps it readable on the fill.
    unsafe fn bolt(&self, cx: f32, cy: f32, size: f32, dim: f32) -> Result<()> {
        const SHAPE: [(f32, f32); 6] = [
            (0.60, 0.00),
            (0.10, 0.57),
            (0.45, 0.57),
            (0.36, 1.00),
            (0.90, 0.41),
            (0.55, 0.41),
        ];
        let (width, left, top) = (size * 0.62, cx - size * 0.31, cy - size / 2.0);
        let point = |(px, py): (f32, f32)| Vector2 {
            X: left + px * width,
            Y: top + py * size,
        };
        unsafe {
            let geometry = self.renderer.d2d.CreatePathGeometry()?;
            let sink = geometry.Open()?;
            sink.BeginFigure(point(SHAPE[0]), D2D1_FIGURE_BEGIN_FILLED);
            for corner in &SHAPE[1..] {
                sink.AddLine(point(*corner));
            }
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            sink.Close()?;
            let contrast = if self.palette.text[0] > 0.5 {
                0x202020
            } else {
                0xFFFFFF
            };
            self.brush.SetColor(&color(theme::with_alpha(
                theme::opaque(contrast),
                0.9 * dim,
            )));
            self.target
                .DrawGeometry(&geometry, self.brush, self.s(1.6), None);
            self.brush
                .SetColor(&color(theme::with_alpha(self.palette.text, dim)));
            self.target.FillGeometry(&geometry, self.brush, None);
            Ok(())
        }
    }
}
