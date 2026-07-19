mod focus;
mod models;
mod providers;
mod strings;

use std::{
    fs,
    io::Write,
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

use chrono::{DateTime, Local, Utc};
use models::{ProviderSnapshot, UsageWindow, WidgetPreferences};
use providers::{CapsulePalette, ProviderDescriptorDto};
use strings::{tray_copy, TrayCopy};
use tauri::{
    image::Image,
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager, State, WindowEvent,
};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tauri_plugin_window_state::{Builder as WindowStateBuilder, StateFlags};

const CACHE_TTL: Duration = Duration::from_secs(30);
const BACKGROUND_REFRESH: Duration = Duration::from_secs(5 * 60);
const TRAY_ICON_HEIGHT: u32 = 36;
const TRAY_CAPSULE_WIDTH: f32 = 80.0;
const TRAY_CAPSULE_HEIGHT: f32 = 34.0;
/// Horizontal budget between two capsules, @2x. Part of it is spent on the icon's outer margin.
const TRAY_CAPSULE_GAP: f32 = 12.0;
/// Transparent margin kept on the far left and far right of the icon.
const TRAY_CAPSULE_MARGIN: f32 = 2.0;
/// A menu bar only has so much room; past this the capsules stop being readable.
const TRAY_CAPSULE_WARN_COUNT: usize = 4;
const TRAY_TIME_DOT_COUNT: u8 = 5;
const COMPACT_WIDGET_WIDTH: f64 = 100.0;
const COMPACT_WIDGET_HEIGHT: f64 = 100.0;

struct AppState {
    client: reqwest::Client,
    preferences: Mutex<WidgetPreferences>,
    preferences_path: PathBuf,
    fetch_lock: tokio::sync::Mutex<()>,
    snapshot_cache: Mutex<Option<(Instant, Vec<ProviderSnapshot>)>>,
}

fn load_preferences(path: &PathBuf) -> WidgetPreferences {
    let parse = |candidate: &PathBuf| {
        fs::read_to_string(candidate)
            .ok()
            .and_then(|raw| serde_json::from_str::<WidgetPreferences>(&raw).ok())
    };
    if let Some(value) = parse(path) {
        return value.normalized();
    }
    let backup = path.with_extension("json.bak");
    if let Some(value) = parse(&backup) {
        eprintln!("preferences recovered from backup");
        return value.normalized();
    }
    WidgetPreferences::default()
}

fn persist_preferences(path: &PathBuf, value: &WidgetPreferences) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|_| "failed to create settings directory".to_string())?;
    }
    let serialized =
        serde_json::to_vec_pretty(value).map_err(|_| "failed to serialize settings".to_string())?;
    let temporary = path.with_extension("json.tmp");
    let backup = path.with_extension("json.bak");
    let mut file = fs::File::create(&temporary)
        .map_err(|_| "failed to create temporary settings file".to_string())?;
    file.write_all(&serialized)
        .and_then(|_| file.sync_all())
        .map_err(|_| "failed to write settings".to_string())?;
    if path.exists() {
        let _ = fs::remove_file(&backup);
        fs::rename(path, &backup).map_err(|_| "failed to back up settings".to_string())?;
    }
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::rename(&backup, path);
        return Err(format!("failed to commit settings: {error}"));
    }
    Ok(())
}

fn preferred_window(snapshot: &ProviderSnapshot) -> Option<(&'static str, &UsageWindow)> {
    snapshot
        .short_window
        .as_ref()
        .map(|window| ("5h", window))
        .or_else(|| {
            snapshot
                .weekly_window
                .as_ref()
                .map(|window| ("week", window))
        })
}

fn rounded_percent(window: &UsageWindow) -> u8 {
    window.remaining_percent.clamp(0.0, 100.0).round() as u8
}

/// The provider the widget should show, by signal strength (§6 of the registry contract):
///
/// 1. The window the user clicked, when it names a provider — via the frontmost app's identity,
///    or via its focused window title once the Accessibility permission is granted. Clicking a
///    window is the user pointing at an assistant; nothing outranks it.
/// 2. Whoever the user last submitted a prompt to (the prompt-history signal). This covers every
///    window the title cannot identify — tmux, renamed tabs, no permission.
/// 3. The old foreground-app default, for a machine where nothing has ever been observed.
#[tauri::command]
fn get_active_provider() -> Option<String> {
    resolve_active_provider(
        focused_provider(),
        providers::activity::active_provider(),
        frontmost_provider,
    )
}

fn resolve_active_provider(
    focused: Option<&'static str>,
    activity: Option<&'static str>,
    fallback: impl FnOnce() -> Option<String>,
) -> Option<String> {
    match focused.or(activity) {
        Some(provider) => Some(provider.to_owned()),
        None => fallback(),
    }
}

/// The provider the focused window belongs to, or `None` when the click names nobody — which is
/// most windows, and the cue to fall through to the prompt-history signal.
///
/// The window title is read only when the app's own identity names no provider, only with the
/// Accessibility permission, and is discarded right after the in-memory hint match: titles carry
/// project and document names, so they are never logged, stored, or returned (see `focus`).
fn focused_provider() -> Option<&'static str> {
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::NSWorkspace;

        let application = NSWorkspace::sharedWorkspace().frontmostApplication()?;
        let bundle_id = application
            .bundleIdentifier()
            .map(|value| value.to_string());
        // Clicking CC's own widget must not move the widget.
        if bundle_id.as_deref() == Some("app.ccquota.desktop") {
            return None;
        }
        let name = application.localizedName().map(|value| value.to_string());
        let named = providers::hinted_provider(&[
            bundle_id.as_deref().unwrap_or(""),
            name.as_deref().unwrap_or(""),
        ])
        .or_else(|| {
            let title = focus::focused_window_title(application.processIdentifier())?;
            providers::hinted_provider(&[&title])
        })?;
        // A provider the user is signed out of has nothing to show; skipping it here lets the
        // prompt-history tier pick someone who does.
        providers::configured()
            .iter()
            .any(|adapter| adapter.descriptor().id == named)
            .then_some(named)
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

fn frontmost_provider() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::NSWorkspace;

        let application = NSWorkspace::sharedWorkspace().frontmostApplication()?;
        let bundle_id = application
            .bundleIdentifier()
            .map(|value| value.to_string());
        let name = application.localizedName().map(|value| value.to_string());
        providers::classify_focus(
            bundle_id.as_deref(),
            name.as_deref(),
            providers::default_focus_provider(),
        )
        .map(str::to_owned)
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

fn snapshot_percent(snapshot: Option<&ProviderSnapshot>) -> Option<u8> {
    snapshot
        .and_then(preferred_window)
        .map(|(_, window)| rounded_percent(window))
}

fn time_remaining_hours(window: &UsageWindow, now: &DateTime<Utc>) -> Option<u8> {
    if window.window_seconds == 0 {
        return None;
    }
    let resets_at = DateTime::parse_from_rfc3339(window.resets_at.as_deref()?)
        .ok()?
        .with_timezone(&Utc);
    let remaining_milliseconds = (resets_at - now).num_milliseconds();
    if remaining_milliseconds <= 0 {
        return Some(0);
    }
    let hours = (remaining_milliseconds as f64 / 3_600_000.0).ceil() as u8;
    Some(hours.clamp(1, TRAY_TIME_DOT_COUNT))
}

fn snapshot_time_hours(snapshot: Option<&ProviderSnapshot>, now: &DateTime<Utc>) -> Option<u8> {
    let (kind, window) = snapshot.and_then(preferred_window)?;
    if kind != "5h" {
        return None;
    }
    time_remaining_hours(window, now)
}

fn rounded_rect_contains(
    x: f32,
    y: f32,
    left: f32,
    top: f32,
    width: f32,
    height: f32,
    radius: f32,
) -> bool {
    let right = left + width;
    let bottom = top + height;
    let radius = radius.max(0.0).min(width * 0.5).min(height * 0.5);
    let min_x = left + radius;
    let max_x = right - radius;
    let min_y = top + radius;
    let max_y = bottom - radius;
    let nearest_x = if min_x < max_x {
        x.clamp(min_x, max_x)
    } else {
        (left + right) * 0.5
    };
    let nearest_y = if min_y < max_y {
        y.clamp(min_y, max_y)
    } else {
        (top + bottom) * 0.5
    };
    let dx = x - nearest_x;
    let dy = y - nearest_y;
    dx * dx + dy * dy <= radius * radius
}

fn coverage(x: u32, y: u32, left: f32, top: f32, width: f32, height: f32, radius: f32) -> f32 {
    let mut hits = 0;
    for sample_y in 0..4 {
        for sample_x in 0..4 {
            let px = x as f32 + (sample_x as f32 + 0.5) / 4.0;
            let py = y as f32 + (sample_y as f32 + 0.5) / 4.0;
            if rounded_rect_contains(px, py, left, top, width, height, radius) {
                hits += 1;
            }
        }
    }
    hits as f32 / 16.0
}

fn blend_pixel(buffer: &mut [u8], width: u32, x: u32, y: u32, color: [u8; 4], coverage: f32) {
    let index = ((y * width + x) * 4) as usize;
    let source_alpha = color[3] as f32 / 255.0 * coverage;
    let destination_alpha = buffer[index + 3] as f32 / 255.0;
    let output_alpha = source_alpha + destination_alpha * (1.0 - source_alpha);
    if output_alpha <= f32::EPSILON {
        return;
    }
    for channel in 0..3 {
        let source = color[channel] as f32 / 255.0;
        let destination = buffer[index + channel] as f32 / 255.0;
        let output = (source * source_alpha
            + destination * destination_alpha * (1.0 - source_alpha))
            / output_alpha;
        buffer[index + channel] = (output * 255.0).round() as u8;
    }
    buffer[index + 3] = (output_alpha * 255.0).round() as u8;
}

#[allow(clippy::too_many_arguments)]
fn draw_rounded_rect(
    buffer: &mut [u8],
    canvas_width: u32,
    left: f32,
    top: f32,
    width: f32,
    height: f32,
    radius: f32,
    color: [u8; 4],
) {
    for y in top.floor().max(0.0) as u32..(top + height).ceil().min(TRAY_ICON_HEIGHT as f32) as u32
    {
        for x in left.floor().max(0.0) as u32..(left + width).ceil().min(canvas_width as f32) as u32
        {
            let amount = coverage(x, y, left, top, width, height, radius);
            if amount > 0.0 {
                blend_pixel(buffer, canvas_width, x, y, color, amount);
            }
        }
    }
}

fn draw_circle(
    buffer: &mut [u8],
    canvas_width: u32,
    center_x: f32,
    center_y: f32,
    radius: f32,
    color: [u8; 4],
) {
    let left = (center_x - radius).floor().max(0.0) as u32;
    let right = (center_x + radius).ceil().min(canvas_width as f32) as u32;
    let top = (center_y - radius).floor().max(0.0) as u32;
    let bottom = (center_y + radius).ceil().min(TRAY_ICON_HEIGHT as f32) as u32;
    let radius_squared = radius * radius;

    for y in top..bottom {
        for x in left..right {
            let mut hits = 0;
            for sample_y in 0..4 {
                for sample_x in 0..4 {
                    let px = x as f32 + (sample_x as f32 + 0.5) / 4.0;
                    let py = y as f32 + (sample_y as f32 + 0.5) / 4.0;
                    let dx = px - center_x;
                    let dy = py - center_y;
                    if dx * dx + dy * dy <= radius_squared {
                        hits += 1;
                    }
                }
            }
            if hits > 0 {
                blend_pixel(buffer, canvas_width, x, y, color, hits as f32 / 16.0);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_line(
    buffer: &mut [u8],
    canvas_width: u32,
    start_x: f32,
    start_y: f32,
    end_x: f32,
    end_y: f32,
    thickness: f32,
    color: [u8; 4],
) {
    let padding = thickness;
    let left = (start_x.min(end_x) - padding).floor().max(0.0) as u32;
    let right = (start_x.max(end_x) + padding)
        .ceil()
        .min(canvas_width as f32) as u32;
    let top = (start_y.min(end_y) - padding).floor().max(0.0) as u32;
    let bottom = (start_y.max(end_y) + padding)
        .ceil()
        .min(TRAY_ICON_HEIGHT as f32) as u32;
    let vector_x = end_x - start_x;
    let vector_y = end_y - start_y;
    let length_squared = vector_x * vector_x + vector_y * vector_y;
    let radius_squared = (thickness * 0.5) * (thickness * 0.5);

    for y in top..bottom {
        for x in left..right {
            let mut hits = 0;
            for sample_y in 0..4 {
                for sample_x in 0..4 {
                    let px = x as f32 + (sample_x as f32 + 0.5) / 4.0;
                    let py = y as f32 + (sample_y as f32 + 0.5) / 4.0;
                    let projection = if length_squared <= f32::EPSILON {
                        0.0
                    } else {
                        ((px - start_x) * vector_x + (py - start_y) * vector_y) / length_squared
                    }
                    .clamp(0.0, 1.0);
                    let nearest_x = start_x + projection * vector_x;
                    let nearest_y = start_y + projection * vector_y;
                    let dx = px - nearest_x;
                    let dy = py - nearest_y;
                    if dx * dx + dy * dy <= radius_squared {
                        hits += 1;
                    }
                }
            }
            if hits > 0 {
                blend_pixel(buffer, canvas_width, x, y, color, hits as f32 / 16.0);
            }
        }
    }
}

fn lerp_color(start: [u8; 4], end: [u8; 4], amount: f32) -> [u8; 4] {
    let amount = amount.clamp(0.0, 1.0);
    let mut output = [0; 4];
    for channel in 0..4 {
        output[channel] = (start[channel] as f32
            + (end[channel] as f32 - start[channel] as f32) * amount)
            .round() as u8;
    }
    output
}

#[derive(Clone, Copy)]
struct CapsuleGeometry {
    left: f32,
    top: f32,
    width: f32,
    height: f32,
    radius: f32,
}

fn draw_capsule_fill(
    buffer: &mut [u8],
    canvas_width: u32,
    geometry: CapsuleGeometry,
    percent: u8,
    top_color: [u8; 4],
    bottom_color: [u8; 4],
) {
    let CapsuleGeometry {
        left,
        top,
        width,
        height,
        radius,
    } = geometry;
    let fill_right = left + width * percent as f32 / 100.0;
    if fill_right <= left {
        return;
    }
    for y in top.floor().max(0.0) as u32..(top + height).ceil().min(TRAY_ICON_HEIGHT as f32) as u32
    {
        let vertical_progress = ((y as f32 + 0.5 - top) / height).clamp(0.0, 1.0);
        let color = lerp_color(top_color, bottom_color, vertical_progress);
        for x in left.floor().max(0.0) as u32..fill_right.ceil().min(canvas_width as f32) as u32 {
            let amount = coverage(x, y, left, top, width, height, radius);
            if amount > 0.0 {
                let horizontal_coverage = (fill_right - x as f32).clamp(0.0, 1.0);
                blend_pixel(
                    buffer,
                    canvas_width,
                    x,
                    y,
                    color,
                    amount * horizontal_coverage,
                );
            }
        }
    }
}

const DIGIT_SEGMENTS: [u8; 10] = [
    0b0011_1111,
    0b0000_0110,
    0b0101_1011,
    0b0100_1111,
    0b0110_0110,
    0b0110_1101,
    0b0111_1101,
    0b0000_0111,
    0b0111_1111,
    0b0110_1111,
];

fn draw_digit(
    buffer: &mut [u8],
    canvas_width: u32,
    left: f32,
    top: f32,
    digit: u8,
    color: [u8; 4],
) {
    let mask = DIGIT_SEGMENTS[digit.min(9) as usize];
    let thickness = 2.3;
    let segments = [
        (left + 2.0, top, 8.0, thickness),
        (left + 9.7, top + 1.7, thickness, 8.2),
        (left + 9.7, top + 11.8, thickness, 8.2),
        (left + 2.0, top + 19.7, 8.0, thickness),
        (left, top + 11.8, thickness, 8.2),
        (left, top + 1.7, thickness, 8.2),
        (left + 2.0, top + 9.9, 8.0, thickness),
    ];
    for (index, (segment_left, segment_top, width, height)) in segments.iter().enumerate() {
        if mask & (1 << index) != 0 {
            draw_rounded_rect(
                buffer,
                canvas_width,
                *segment_left,
                *segment_top,
                *width,
                *height,
                thickness * 0.5,
                color,
            );
        }
    }
}

fn draw_percent_symbol(buffer: &mut [u8], canvas_width: u32, left: f32, top: f32, color: [u8; 4]) {
    draw_circle(buffer, canvas_width, left + 2.0, top + 3.4, 1.85, color);
    draw_line(
        buffer,
        canvas_width,
        left + 8.6,
        top + 1.7,
        left + 1.4,
        top + 20.1,
        1.85,
        color,
    );
    draw_circle(buffer, canvas_width, left + 7.7, top + 18.4, 1.85, color);
}

fn tray_font() -> Option<&'static fontdue::Font> {
    static FONT: OnceLock<Option<fontdue::Font>> = OnceLock::new();
    FONT.get_or_init(|| {
        [
            "/System/Library/Fonts/SFCompact.ttf",
            "/System/Library/Fonts/SFNS.ttf",
        ]
        .iter()
        .find_map(|path| {
            fs::read(path).ok().and_then(|bytes| {
                fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default()).ok()
            })
        })
    })
    .as_ref()
}

fn draw_system_percent_label(
    buffer: &mut [u8],
    canvas_width: u32,
    capsule_left: f32,
    percent: Option<u8>,
) -> bool {
    use fontdue::layout::{
        CoordinateSystem, HorizontalAlign, Layout, LayoutSettings, TextStyle, VerticalAlign,
    };

    let Some(font) = tray_font() else {
        return false;
    };
    let text = percent
        .map(|value| format!("{value}%"))
        .unwrap_or_else(|| "—".to_string());
    let mut layout = Layout::new(CoordinateSystem::PositiveYDown);
    layout.reset(&LayoutSettings {
        x: capsule_left,
        y: 1.0,
        max_width: Some(TRAY_CAPSULE_WIDTH),
        max_height: Some(TRAY_CAPSULE_HEIGHT),
        horizontal_align: HorizontalAlign::Center,
        vertical_align: VerticalAlign::Middle,
        ..LayoutSettings::default()
    });
    layout.append(&[font], &TextStyle::new(&text, 22.5, 0));
    if layout.glyphs().is_empty() {
        return false;
    }

    let text_color = [255, 255, 255, 248];
    for glyph in layout.glyphs() {
        let (_, bitmap) = font.rasterize_config(glyph.key);
        let origin_x = glyph.x.floor() as i32;
        let origin_y = glyph.y.floor() as i32;
        for row in 0..glyph.height {
            for column in 0..glyph.width {
                let alpha = bitmap[row * glyph.width + column] as f32 / 255.0;
                if alpha <= 0.0 {
                    continue;
                }
                let x = origin_x + column as i32;
                let y = origin_y + row as i32;
                if x >= 0 && x < canvas_width as i32 && y >= 0 && y < TRAY_ICON_HEIGHT as i32 {
                    blend_pixel(buffer, canvas_width, x as u32, y as u32, text_color, alpha);
                    if x + 1 < canvas_width as i32 {
                        blend_pixel(
                            buffer,
                            canvas_width,
                            (x + 1) as u32,
                            y as u32,
                            text_color,
                            alpha * 0.22,
                        );
                    }
                }
            }
        }
    }
    true
}

fn draw_percent_label(
    buffer: &mut [u8],
    canvas_width: u32,
    capsule_left: f32,
    percent: Option<u8>,
) {
    if draw_system_percent_label(buffer, canvas_width, capsule_left, percent) {
        return;
    }
    let text_color = [255, 255, 255, 244];
    let top = 7.1;
    let Some(percent) = percent else {
        let dash_width = 12.0;
        let left = capsule_left + (TRAY_CAPSULE_WIDTH - dash_width) * 0.5;
        draw_rounded_rect(
            buffer,
            canvas_width,
            left,
            16.9,
            dash_width,
            2.15,
            1.08,
            text_color,
        );
        return;
    };

    let digits = percent.to_string();
    let digit_width = 12.0;
    let digit_gap = 2.0;
    let suffix_gap = 2.3;
    let percent_width = 9.6;
    let digits_width =
        digits.len() as f32 * digit_width + digits.len().saturating_sub(1) as f32 * digit_gap;
    let total_width = digits_width + suffix_gap + percent_width;
    let mut left = capsule_left + (TRAY_CAPSULE_WIDTH - total_width) * 0.5;
    for character in digits.bytes() {
        draw_digit(
            buffer,
            canvas_width,
            left,
            top,
            character.saturating_sub(b'0'),
            text_color,
        );
        left += digit_width + digit_gap;
    }
    draw_percent_symbol(
        buffer,
        canvas_width,
        left - digit_gap + suffix_gap,
        top,
        text_color,
    );
}

fn draw_capsule_time_dots(buffer: &mut [u8], canvas_width: u32, left: f32, hours: u8) {
    let spacing = 6.0;
    let start_x = left + TRAY_CAPSULE_WIDTH * 0.5
        - spacing * (TRAY_TIME_DOT_COUNT.saturating_sub(1) as f32) * 0.5;
    let center_y = 32.0;
    let lit_hours = hours.min(TRAY_TIME_DOT_COUNT);

    for index in 0..TRAY_TIME_DOT_COUNT {
        let center_x = start_x + index as f32 * spacing;
        draw_circle(
            buffer,
            canvas_width,
            center_x,
            center_y,
            1.25,
            [255, 255, 255, 48],
        );
        if index < lit_hours {
            draw_circle(
                buffer,
                canvas_width,
                center_x,
                center_y,
                1.25,
                [255, 255, 255, 236],
            );
        }
    }
}

fn draw_tray_capsule(
    buffer: &mut [u8],
    canvas_width: u32,
    left: f32,
    percent: Option<u8>,
    time_hours: Option<u8>,
    palette: CapsulePalette,
) {
    let top = 1.0;
    let radius = 17.0;
    draw_rounded_rect(
        buffer,
        canvas_width,
        left,
        top,
        TRAY_CAPSULE_WIDTH,
        TRAY_CAPSULE_HEIGHT,
        radius,
        palette.border,
    );
    let inner_left = left + 1.2;
    let inner_top = top + 1.2;
    let inner_width = TRAY_CAPSULE_WIDTH - 2.4;
    let inner_height = TRAY_CAPSULE_HEIGHT - 2.4;
    draw_rounded_rect(
        buffer,
        canvas_width,
        inner_left,
        inner_top,
        inner_width,
        inner_height,
        radius - 1.2,
        palette.track,
    );
    if let Some(value) = percent {
        draw_capsule_fill(
            buffer,
            canvas_width,
            CapsuleGeometry {
                left: inner_left,
                top: inner_top,
                width: inner_width,
                height: inner_height,
                radius: radius - 1.2,
            },
            value,
            palette.fill_top,
            palette.fill_bottom,
        );
    }
    draw_percent_label(buffer, canvas_width, left, percent);
    if let Some(value) = time_hours {
        draw_capsule_time_dots(buffer, canvas_width, left, value);
    }
}

/// One capsule's worth of already-resolved drawing input.
#[derive(Clone, Copy)]
struct TrayCapsule {
    palette: CapsulePalette,
    percent: Option<u8>,
    time_hours: Option<u8>,
}

/// Icon width for `count` capsules, per the provider registry contract §1.5.
/// Two capsules must come out at the historical 172px.
fn tray_icon_width(count: usize) -> u32 {
    let count = count.max(1) as f32;
    (count * TRAY_CAPSULE_WIDTH + (count - 1.0) * TRAY_CAPSULE_GAP).round() as u32
}

/// Left edge of capsule `index`. The outer margin is taken out of the gap budget so the first and
/// last capsules keep `TRAY_CAPSULE_MARGIN` of breathing room against the icon edges.
fn tray_capsule_left(index: usize, count: usize) -> f32 {
    if count <= 1 {
        return 0.0;
    }
    let inner_gap = TRAY_CAPSULE_GAP - 2.0 * TRAY_CAPSULE_MARGIN / (count - 1) as f32;
    TRAY_CAPSULE_MARGIN + index as f32 * (TRAY_CAPSULE_WIDTH + inner_gap)
}

fn render_tray_capsules(capsules: &[TrayCapsule]) -> Vec<u8> {
    let width = tray_icon_width(capsules.len());
    let mut rgba = vec![0; (width * TRAY_ICON_HEIGHT * 4) as usize];
    for (index, capsule) in capsules.iter().enumerate() {
        draw_tray_capsule(
            &mut rgba,
            width,
            tray_capsule_left(index, capsules.len()),
            capsule.percent,
            capsule.time_hours,
            capsule.palette,
        );
    }
    rgba
}

/// Turns the snapshots the app currently holds into the menu bar bitmap.
/// Returns the pixels alongside the width, which now varies with the provider count.
fn tray_icon_rgba(snapshots: &[ProviderSnapshot], now: &DateTime<Utc>) -> (Vec<u8>, u32) {
    if snapshots.len() > TRAY_CAPSULE_WARN_COUNT {
        eprintln!(
            "menu bar icon is showing {} providers; it may crowd the menu bar",
            snapshots.len()
        );
    }
    let capsules: Vec<TrayCapsule> = snapshots
        .iter()
        .filter_map(|snapshot| {
            let descriptor = providers::find(&snapshot.provider)?.descriptor();
            Some(TrayCapsule {
                palette: descriptor.palette,
                percent: snapshot_percent(Some(snapshot)),
                time_hours: snapshot_time_hours(Some(snapshot), now),
            })
        })
        .collect();
    (
        render_tray_capsules(&capsules),
        tray_icon_width(capsules.len()),
    )
}

/// Placeholder icon for before the first fetch lands.
fn empty_tray_icon_rgba() -> (Vec<u8>, u32) {
    let capsules: Vec<TrayCapsule> = providers::active()
        .into_iter()
        .map(|adapter| TrayCapsule {
            palette: adapter.descriptor().palette,
            percent: None,
            time_hours: None,
        })
        .collect();
    (
        render_tray_capsules(&capsules),
        tray_icon_width(capsules.len()),
    )
}

fn reset_label(value: Option<&str>, copy: &TrayCopy) -> String {
    let Some(value) = value else {
        return copy.reset_unknown.into();
    };
    DateTime::parse_from_rfc3339(value)
        .map(|date| {
            date.with_timezone(&Local)
                .format(copy.reset_format)
                .to_string()
        })
        .unwrap_or_else(|_| copy.reset_unknown.into())
}

fn provider_menu_line(
    snapshot: Option<&ProviderSnapshot>,
    display_name: &str,
    copy: &TrayCopy,
) -> String {
    let Some(snapshot) = snapshot else {
        return format!("{display_name} · {}", copy.loading);
    };
    if let Some((kind, window)) = preferred_window(snapshot) {
        let stale = if snapshot.status == "stale" {
            copy.stale_suffix
        } else {
            ""
        };
        return format!(
            "{display_name} · {kind} {}% · {}{stale}",
            rounded_percent(window),
            reset_label(window.resets_at.as_deref(), copy)
        );
    }
    let message = snapshot.message.as_deref().unwrap_or(copy.unavailable);
    format!("{display_name} · {message}")
}

fn cached_snapshots(state: &State<'_, AppState>) -> Vec<ProviderSnapshot> {
    state
        .snapshot_cache
        .lock()
        .ok()
        .and_then(|cache| cache.as_ref().map(|(_, values)| values.clone()))
        .unwrap_or_default()
}

fn update_tray_ui(app: &AppHandle, snapshots: &[ProviderSnapshot]) -> Result<(), String> {
    let tray = app
        .tray_by_id("main")
        .ok_or_else(|| "menu bar item missing".to_string())?;
    let state = app.state::<AppState>();
    let preferences = state
        .preferences
        .lock()
        .map_err(|_| "settings unavailable".to_string())?
        .clone();
    let copy = tray_copy(&preferences.language);
    // Registry order drives both the tooltip and the detail rows; the snapshots are only looked up.
    let ordered: Vec<(
        &'static providers::ProviderDescriptor,
        Option<&ProviderSnapshot>,
    )> = providers::active()
        .into_iter()
        .map(|adapter| {
            let descriptor = adapter.descriptor();
            let snapshot = snapshots.iter().find(|item| item.provider == descriptor.id);
            (descriptor, snapshot)
        })
        .collect();
    let tooltip = ordered
        .iter()
        .map(|(descriptor, snapshot)| provider_menu_line(*snapshot, descriptor.display_name, copy))
        .collect::<Vec<_>>()
        .join("\n");

    let detail_items = ordered
        .iter()
        .map(|(descriptor, snapshot)| {
            MenuItem::with_id(
                app,
                format!("{}_detail", descriptor.id),
                provider_menu_line(*snapshot, descriptor.display_name, copy),
                false,
                None::<&str>,
            )
            .map_err(|error| error.to_string())
        })
        .collect::<Result<Vec<_>, String>>()?;
    let separator_top = PredefinedMenuItem::separator(app).map_err(|error| error.to_string())?;
    let toggle_widget = CheckMenuItem::with_id(
        app,
        "toggle_widget",
        copy.show_widget,
        true,
        preferences.widget_visible,
        None::<&str>,
    )
    .map_err(|error| error.to_string())?;
    let always_on_top = CheckMenuItem::with_id(
        app,
        "always_on_top",
        copy.always_on_top,
        true,
        preferences.always_on_top,
        None::<&str>,
    )
    .map_err(|error| error.to_string())?;
    let unlock = MenuItem::with_id(
        app,
        "unlock",
        copy.disable_click_through,
        preferences.locked,
        None::<&str>,
    )
    .map_err(|error| error.to_string())?;
    let refresh = MenuItem::with_id(app, "refresh", copy.refresh_now, true, None::<&str>)
        .map_err(|error| error.to_string())?;
    let language = MenuItem::with_id(app, "language", copy.switch_language, true, None::<&str>)
        .map_err(|error| error.to_string())?;
    let follow_trusted = focus::trusted();
    let follow_window = CheckMenuItem::with_id(
        app,
        "follow_window",
        copy.follow_window,
        !follow_trusted,
        follow_trusted,
        None::<&str>,
    )
    .map_err(|error| error.to_string())?;
    let autostart_enabled = app.autolaunch().is_enabled().unwrap_or(false);
    let autostart = CheckMenuItem::with_id(
        app,
        "autostart",
        copy.launch_at_login,
        true,
        autostart_enabled,
        None::<&str>,
    )
    .map_err(|error| error.to_string())?;
    let separator_bottom = PredefinedMenuItem::separator(app).map_err(|error| error.to_string())?;
    let quit = MenuItem::with_id(app, "quit", copy.quit, true, None::<&str>)
        .map_err(|error| error.to_string())?;
    let mut items: Vec<&dyn tauri::menu::IsMenuItem<tauri::Wry>> = Vec::new();
    for item in &detail_items {
        items.push(item);
    }
    for item in [
        &separator_top as &dyn tauri::menu::IsMenuItem<tauri::Wry>,
        &toggle_widget,
        &always_on_top,
        &unlock,
        &refresh,
        &language,
        &follow_window,
        &autostart,
        &separator_bottom,
        &quit,
    ] {
        items.push(item);
    }
    let menu = Menu::with_items(app, &items).map_err(|error| error.to_string())?;

    let now = Utc::now();
    // A provider with no reading yet still gets its capsule, drawn as the "—" placeholder.
    let ordered_snapshots: Vec<ProviderSnapshot> = ordered
        .iter()
        .map(|(descriptor, snapshot)| {
            snapshot.cloned().unwrap_or_else(|| {
                ProviderSnapshot::failure_for(
                    descriptor.id,
                    &descriptor.display_name.to_uppercase(),
                    "unavailable",
                    "",
                )
            })
        })
        .collect();
    let (pixels, width) = tray_icon_rgba(&ordered_snapshots, &now);
    tray.set_icon_with_as_template(
        Some(Image::new_owned(pixels, width, TRAY_ICON_HEIGHT)),
        false,
    )
    .map_err(|error| error.to_string())?;
    tray.set_tooltip(Some(tooltip))
        .map_err(|error| error.to_string())?;
    tray.set_menu(Some(menu))
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn refresh_tray_from_cache(app: &AppHandle) {
    let state = app.state::<AppState>();
    let values = cached_snapshots(&state);
    let _ = update_tray_ui(app, &values);
}

fn unavailable_snapshots(message: &str) -> Vec<ProviderSnapshot> {
    // `active`, not `all`: during the cold-start window where a fetch is in flight and the cache
    // is still empty, a colliding request must not invent cards for providers the user never
    // signed into. On a machine with no logins at all, `active` still falls back to the full
    // registry so the capsules can explain themselves.
    providers::active()
        .iter()
        .map(|adapter| {
            let descriptor = adapter.descriptor();
            ProviderSnapshot::failure_for(
                descriptor.id,
                &descriptor.display_name.to_uppercase(),
                "unavailable",
                message,
            )
        })
        .collect()
}

/// Mirrors `isSnapshotDisplayable` in `src/lib/format.ts`: past this age the last good reading is
/// no longer worth showing. Without it the tray keeps rendering a confident percentage from an
/// arbitrarily old fetch while the widget has already blanked out.
///
/// A day, not minutes: Kimi's access token lives about fifteen minutes and its CLI renews it only
/// when the user returns, so any shorter window made the capsule vanish over lunch. The reading is
/// dimmed and dated the whole time; a day-old number marked stale beats an empty capsule, and only
/// a full day of silence earns the empty state.
const MAX_STALE_SECONDS: i64 = 24 * 60 * 60;

fn is_within_stale_window(snapshot: &ProviderSnapshot, now: &DateTime<Utc>) -> bool {
    DateTime::parse_from_rfc3339(&snapshot.updated_at)
        .map(|updated| (*now - updated.with_timezone(&Utc)).num_seconds() <= MAX_STALE_SECONDS)
        .unwrap_or(false)
}

fn merge_snapshots(
    current: &[ProviderSnapshot],
    incoming: Vec<ProviderSnapshot>,
    now: &DateTime<Utc>,
) -> Vec<ProviderSnapshot> {
    incoming
        .into_iter()
        .map(|next| {
            // Every failure keeps the last good reading, whatever its status. Statuses are not a
            // reliable signal of permanence: an expired short-lived token reports `signed_out`
            // while the provider's own CLI is about to renew it, and letting that erase the
            // numbers made the capsule vanish instead of dimming. The age cutoff below is what
            // bounds how long a reading may be shown; the failure's message still rides along.
            if next.status == "ok" {
                return next;
            }
            let Some(mut previous) = current
                .iter()
                .find(|item| item.provider == next.provider && preferred_window(item).is_some())
                .filter(|item| is_within_stale_window(item, now))
                .cloned()
            else {
                return next;
            };
            previous.status = "stale".into();
            previous.message = next.message;
            previous
        })
        .collect()
}

/// Ceiling on how long one provider may hold up a refresh round.
///
/// The HTTP client has its own, shorter timeout, so this is the backstop for everything it does
/// not cover — a credential helper that never returns, a body that trickles in forever. It sits
/// above the sum of an adapter's internal timeouts so the adapter's own, better-worded failure is
/// what normally surfaces. Without it one wedged provider stalls the whole `join_all` and the
/// menu bar freezes on every provider at once, which is how a single slow endpoint (Kimi has
/// answered a plain request with a 20 second hang and an HTML `504`) becomes an app-wide outage.
const PROVIDER_FETCH_BUDGET: Duration = Duration::from_secs(20);

/// Failure recorded when a provider blows the budget. Ordinary failure shape, so the last good
/// reading is retained as stale exactly like a 401 or a 504 would be.
fn timed_out_snapshot(descriptor: &providers::ProviderDescriptor) -> ProviderSnapshot {
    ProviderSnapshot::failure_for(
        descriptor.id,
        &descriptor.display_name.to_uppercase(),
        "unavailable",
        "Quota service did not respond in time. It will retry automatically.",
    )
}

async fn fetch_within_budget(
    adapter: &'static dyn providers::ProviderAdapter,
    client: &reqwest::Client,
) -> ProviderSnapshot {
    tokio::time::timeout(PROVIDER_FETCH_BUDGET, adapter.fetch_snapshot(client))
        .await
        .unwrap_or_else(|_| timed_out_snapshot(adapter.descriptor()))
}

async fn fetch_snapshots_for_app(app: &AppHandle, force: bool) -> Vec<ProviderSnapshot> {
    let state = app.state::<AppState>();
    if !force {
        if let Ok(cache) = state.snapshot_cache.lock() {
            if let Some((time, values)) = &*cache {
                if time.elapsed() < CACHE_TTL {
                    return values.clone();
                }
            }
        }
    }

    let _guard = match state.fetch_lock.try_lock() {
        Ok(guard) => guard,
        Err(_) => {
            if let Ok(cache) = state.snapshot_cache.lock() {
                if let Some((_, values)) = &*cache {
                    return values.clone();
                }
            }
            return unavailable_snapshots("Quota refresh is already running.");
        }
    };

    if !force {
        if let Ok(cache) = state.snapshot_cache.lock() {
            if let Some((time, values)) = &*cache {
                if time.elapsed() < CACHE_TTL {
                    return values.clone();
                }
            }
        }
    }

    let incoming = futures_util::future::join_all(
        providers::active()
            .into_iter()
            .map(|adapter| fetch_within_budget(adapter, &state.client)),
    )
    .await;
    let current = state
        .snapshot_cache
        .lock()
        .ok()
        .and_then(|cache| cache.as_ref().map(|(_, values)| values.clone()))
        .unwrap_or_default();
    let values = merge_snapshots(&current, incoming, &Utc::now());
    if let Ok(mut cache) = state.snapshot_cache.lock() {
        *cache = Some((Instant::now(), values.clone()));
    }
    let _ = update_tray_ui(app, &values);
    let _ = app.emit_to("widget", "snapshots-changed", values.clone());
    values
}

#[tauri::command]
async fn get_snapshots(app: AppHandle) -> Result<Vec<ProviderSnapshot>, String> {
    Ok(fetch_snapshots_for_app(&app, false).await)
}

#[tauri::command]
async fn refresh_snapshots(app: AppHandle) -> Result<Vec<ProviderSnapshot>, String> {
    Ok(fetch_snapshots_for_app(&app, true).await)
}

/// Single source of provider identity for the UI: ids, names, badges and accent colours all come
/// from the Rust registry so nothing has to be kept in sync inside the frontend.
#[tauri::command]
fn get_provider_descriptors() -> Vec<ProviderDescriptorDto> {
    providers::descriptor_dtos()
}

#[tauri::command]
fn get_preferences(state: State<'_, AppState>) -> Result<WidgetPreferences, String> {
    state
        .preferences
        .lock()
        .map(|value| value.clone())
        .map_err(|_| "settings unavailable".into())
}

fn apply_widget_visibility(app: &AppHandle, visible: bool) -> Result<(), String> {
    let window = app
        .get_webview_window("widget")
        .ok_or_else(|| "widget window missing".to_string())?;
    if visible {
        window
            .show()
            .map_err(|error| format!("failed to show widget: {error}"))?;
        let _ = window.unminimize();
        let _ = window.set_focus();
        Ok(())
    } else {
        window
            .hide()
            .map_err(|error| format!("failed to hide widget: {error}"))
    }
}

fn apply_lock(app: &AppHandle, locked: bool) -> Result<(), String> {
    let window = app
        .get_webview_window("widget")
        .ok_or_else(|| "widget window missing".to_string())?;
    window
        .set_ignore_cursor_events(locked)
        .map_err(|_| "failed to toggle click-through".to_string())
}

fn restore_preferences(app: &AppHandle, state: &State<'_, AppState>, value: &WidgetPreferences) {
    let _ = persist_preferences(&state.preferences_path, value);
    if let Some(window) = app.get_webview_window("widget") {
        let _ = window.set_always_on_top(value.always_on_top);
    }
    let _ = apply_lock(app, value.locked);
    let _ = apply_widget_visibility(app, value.widget_visible);
}

fn commit_preferences(
    app: &AppHandle,
    state: &State<'_, AppState>,
    previous: &WidgetPreferences,
    next: WidgetPreferences,
) -> Result<WidgetPreferences, String> {
    persist_preferences(&state.preferences_path, &next)?;
    let window = app
        .get_webview_window("widget")
        .ok_or_else(|| "widget window missing".to_string())?;
    if let Err(error) = window.set_always_on_top(next.always_on_top) {
        restore_preferences(app, state, previous);
        return Err(format!("failed to toggle always-on-top: {error}"));
    }
    if let Err(error) = apply_lock(app, next.locked) {
        restore_preferences(app, state, previous);
        return Err(error);
    }
    if let Err(error) = apply_widget_visibility(app, next.widget_visible) {
        restore_preferences(app, state, previous);
        return Err(error);
    }
    *state
        .preferences
        .lock()
        .map_err(|_| "settings unavailable".to_string())? = next.clone();
    let _ = app.emit_to("widget", "preferences-changed", next.clone());
    refresh_tray_from_cache(app);
    Ok(next)
}

#[tauri::command]
fn set_preferences(
    preferences: WidgetPreferences,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let previous = state
        .preferences
        .lock()
        .map_err(|_| "settings unavailable".to_string())?
        .clone();
    commit_preferences(&app, &state, &previous, preferences.normalized())?;
    Ok(())
}

#[tauri::command]
fn set_widget_locked(
    locked: bool,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<WidgetPreferences, String> {
    let previous = state
        .preferences
        .lock()
        .map_err(|_| "settings unavailable".to_string())?
        .clone();
    let mut next = previous.clone();
    next.locked = locked;
    commit_preferences(&app, &state, &previous, next)
}

#[tauri::command]
fn set_widget_always_on_top(
    always_on_top: bool,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<WidgetPreferences, String> {
    let previous = state
        .preferences
        .lock()
        .map_err(|_| "settings unavailable".to_string())?
        .clone();
    let mut next = previous.clone();
    next.always_on_top = always_on_top;
    commit_preferences(&app, &state, &previous, next)
}

#[tauri::command]
fn set_widget_visible(
    visible: bool,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<WidgetPreferences, String> {
    let previous = state
        .preferences
        .lock()
        .map_err(|_| "settings unavailable".to_string())?
        .clone();
    let mut next = previous.clone();
    next.widget_visible = visible;
    commit_preferences(&app, &state, &previous, next)
}

fn update_preferences_from_tray(app: &AppHandle, mutate: impl FnOnce(&mut WidgetPreferences)) {
    let state = app.state::<AppState>();
    let previous = match state.preferences.lock() {
        Ok(value) => value.clone(),
        Err(_) => return,
    };
    let mut next = previous.clone();
    mutate(&mut next);
    let _ = commit_preferences(app, &state, &previous, next.normalized());
}

fn handle_tray_menu(app: &AppHandle, id: &str) {
    match id {
        "toggle_widget" => update_preferences_from_tray(app, |preferences| {
            preferences.widget_visible = !preferences.widget_visible;
        }),
        "always_on_top" => update_preferences_from_tray(app, |preferences| {
            preferences.always_on_top = !preferences.always_on_top;
        }),
        "unlock" => update_preferences_from_tray(app, |preferences| {
            preferences.locked = false;
        }),
        "refresh" => {
            let handle = app.clone();
            tauri::async_runtime::spawn(async move {
                let _ = fetch_snapshots_for_app(&handle, true).await;
            });
        }
        "language" => update_preferences_from_tray(app, |preferences| {
            preferences.language = if preferences.language == "en" {
                "zh-CN".into()
            } else {
                "en".into()
            };
        }),
        "follow_window" => {
            // Shows the system's Accessibility dialog; the checkbox reflects the granted state
            // the next time the menu is rebuilt (granting may need an app restart to bite).
            focus::request_trust();
            refresh_tray_from_cache(app);
        }
        "autostart" => {
            let manager = app.autolaunch();
            let enabled = manager.is_enabled().unwrap_or(false);
            if if enabled {
                manager.disable()
            } else {
                manager.enable()
            }
            .is_ok()
            {
                refresh_tray_from_cache(app);
            }
        }
        "quit" => app.exit(0),
        _ => {}
    }
}

fn setup_tray(app: &tauri::App, language: &str) -> tauri::Result<()> {
    let copy = tray_copy(language);
    let descriptors: Vec<&'static providers::ProviderDescriptor> = providers::active()
        .into_iter()
        .map(|adapter| adapter.descriptor())
        .collect();
    let detail_items = descriptors
        .iter()
        .map(|descriptor| {
            MenuItem::with_id(
                app,
                format!("{}_detail", descriptor.id),
                provider_menu_line(None, descriptor.display_name, copy),
                false,
                None::<&str>,
            )
        })
        .collect::<tauri::Result<Vec<_>>>()?;
    let separator_top = PredefinedMenuItem::separator(app)?;
    let toggle_widget = CheckMenuItem::with_id(
        app,
        "toggle_widget",
        copy.show_widget,
        true,
        false,
        None::<&str>,
    )?;
    let refresh = MenuItem::with_id(app, "refresh", copy.refresh_now, true, None::<&str>)?;
    let separator_bottom = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", copy.quit, true, None::<&str>)?;
    let mut items: Vec<&dyn tauri::menu::IsMenuItem<tauri::Wry>> = Vec::new();
    for item in &detail_items {
        items.push(item);
    }
    for item in [
        &separator_top as &dyn tauri::menu::IsMenuItem<tauri::Wry>,
        &toggle_widget,
        &refresh,
        &separator_bottom,
        &quit,
    ] {
        items.push(item);
    }
    let menu = Menu::with_items(app, &items)?;
    let (pixels, width) = empty_tray_icon_rgba();
    let tooltip = format!(
        "CC Quota · {}",
        descriptors
            .iter()
            .map(|descriptor| descriptor.display_name)
            .collect::<Vec<_>>()
            .join(" & ")
    );
    TrayIconBuilder::with_id("main")
        .icon(Image::new_owned(pixels, width, TRAY_ICON_HEIGHT))
        .icon_as_template(false)
        .menu(&menu)
        .show_menu_on_left_click(true)
        .tooltip(tooltip)
        .on_menu_event(|app, event| handle_tray_menu(app, event.id.as_ref()))
        .build(app)?;
    Ok(())
}

pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(state) = app.try_state::<AppState>() {
                let should_show = state
                    .preferences
                    .lock()
                    .map(|preferences| preferences.widget_visible)
                    .unwrap_or(false);
                if should_show {
                    let _ = apply_widget_visibility(app, true);
                }
            }
        }))
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(
            WindowStateBuilder::default()
                .with_state_flags(StateFlags::POSITION)
                .build(),
        )
        .setup(|app| {
            #[cfg(target_os = "macos")]
            {
                let _ = app
                    .handle()
                    .set_activation_policy(tauri::ActivationPolicy::Accessory);
                let _ = app.handle().set_dock_visibility(false);
            }

            let data_dir = app.path().app_config_dir()?;
            let preferences_path = data_dir.join("preferences.json");
            let legacy_preferences_path = data_dir.parent().map(|parent| {
                parent
                    .join("app.quotafloat.desktop")
                    .join("preferences.json")
            });
            let preferences = if preferences_path.exists()
                || preferences_path.with_extension("json.bak").exists()
            {
                load_preferences(&preferences_path)
            } else if let Some(legacy_path) = legacy_preferences_path
                .as_ref()
                .filter(|path| path.exists() || path.with_extension("json.bak").exists())
            {
                let migrated = load_preferences(legacy_path);
                let _ = persist_preferences(&preferences_path, &migrated);
                migrated
            } else {
                WidgetPreferences::default()
            };
            let client = reqwest::Client::builder()
                // Covers connect, response and body read. Kept well under
                // `PROVIDER_FETCH_BUDGET` so a gateway that hangs (Kimi's has held a request open
                // for twenty seconds before answering) is abandoned with a proper message rather
                // than caught by the backstop.
                .timeout(Duration::from_secs(10))
                .connect_timeout(Duration::from_secs(5))
                .redirect(reqwest::redirect::Policy::none())
                // Derived at compile time: `check-version.mjs` only guards the three manifests,
                // so a hardcoded string here would drift silently on the next release.
                .user_agent(concat!("CCQuota/", env!("CARGO_PKG_VERSION")))
                .build()
                .expect("static HTTP client configuration must be valid");
            app.manage(AppState {
                client,
                preferences: Mutex::new(preferences.clone()),
                preferences_path,
                fetch_lock: tokio::sync::Mutex::new(()),
                snapshot_cache: Mutex::new(None),
            });

            if setup_tray(app, &preferences.language).is_err() {
                eprintln!("tray setup failed; enabling taskbar fallback");
                if let Some(window) = app.get_webview_window("widget") {
                    let _ = window.set_skip_taskbar(false);
                }
            }

            if let Some(window) = app.get_webview_window("widget") {
                let _ = window.set_size(tauri::LogicalSize::new(
                    COMPACT_WIDGET_WIDTH,
                    COMPACT_WIDGET_HEIGHT,
                ));
                let _ = window.set_background_color(Some(tauri::window::Color(0, 0, 0, 0)));
                let _ = window.set_always_on_top(preferences.always_on_top);
            }
            // Subscribe before the widget's first poll, so early CLI activity is not missed.
            providers::activity::start();
            let _ = apply_lock(app.handle(), preferences.locked);
            let _ = apply_widget_visibility(app.handle(), preferences.widget_visible);
            refresh_tray_from_cache(app.handle());

            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let _ = fetch_snapshots_for_app(&handle, true).await;
                loop {
                    tokio::time::sleep(BACKGROUND_REFRESH).await;
                    let _ = fetch_snapshots_for_app(&handle, true).await;
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_snapshots,
            refresh_snapshots,
            get_preferences,
            set_preferences,
            set_widget_locked,
            set_widget_always_on_top,
            set_widget_visible,
            get_active_provider,
            get_provider_descriptors
        ])
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let app = window.app_handle();
                update_preferences_from_tray(app, |preferences| {
                    preferences.widget_visible = false;
                });
            }
        })
        .build(tauri::generate_context!())
        .expect("failed to build CC");
    app.run(|app_handle, event| {
        if matches!(event, tauri::RunEvent::Resumed) {
            let handle = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                let _ = fetch_snapshots_for_app(&handle, true).await;
            });
        }
    });
}

#[cfg(test)]
mod active_provider_tests {
    use super::resolve_active_provider;
    use std::cell::Cell;

    /// Clicking a window is the user pointing at an assistant; it outranks who they last typed
    /// to, or switching windows would not move the widget until the next prompt.
    #[test]
    fn the_focused_window_outranks_prompt_history() {
        let asked = Cell::new(false);
        let resolved = resolve_active_provider(Some("alpha"), Some("beta"), || {
            asked.set(true);
            Some("gamma".to_owned())
        });
        assert_eq!(resolved.as_deref(), Some("alpha"));
        assert!(!asked.get());
    }

    /// Observed prompt activity answers the question when the click names nobody; the frontmost
    /// app is not even consulted, which is the point — inside one terminal it would give the
    /// wrong provider.
    #[test]
    fn activity_wins_and_the_foreground_app_is_never_asked() {
        let asked = Cell::new(false);
        let resolved = resolve_active_provider(None, Some("alpha"), || {
            asked.set(true);
            Some("beta".to_owned())
        });
        assert_eq!(resolved.as_deref(), Some("alpha"));
        assert!(!asked.get());
    }

    /// With no prompt ever observed there is nothing to rank, so the previous foreground-app
    /// behaviour stands rather than the widget guessing.
    #[test]
    fn falls_back_to_the_foreground_app_when_nothing_was_observed() {
        assert_eq!(
            resolve_active_provider(None, None, || Some("beta".to_owned())).as_deref(),
            Some("beta")
        );
        assert_eq!(resolve_active_provider(None, None, || None), None);
    }
}

#[cfg(test)]
mod tray_icon_tests {
    use super::{
        empty_tray_icon_rgba, merge_snapshots, providers, render_tray_capsules,
        time_remaining_hours, timed_out_snapshot, tray_capsule_left, tray_icon_rgba,
        tray_icon_width, ProviderSnapshot, TrayCapsule, UsageWindow, TRAY_CAPSULE_WIDTH,
        TRAY_ICON_HEIGHT,
    };
    use chrono::{TimeZone, Utc};

    /// Capsule counts every layout test runs against, so nothing silently assumes "exactly two".
    const CAPSULE_COUNTS: [usize; 4] = [1, 2, 3, 4];

    /// The width and capsule origins the app shipped with. Two capsules must still land here
    /// pixel for pixel, otherwise the menu bar look changed.
    const HISTORICAL_WIDTH: u32 = 172;
    const HISTORICAL_LEFTS: [f32; 2] = [2.0, 90.0];

    fn palette(index: usize) -> providers::CapsulePalette {
        let registry = providers::all();
        registry[index % registry.len()].descriptor().palette
    }

    fn capsules(values: &[(Option<u8>, Option<u8>)]) -> Vec<TrayCapsule> {
        values
            .iter()
            .enumerate()
            .map(|(index, (percent, time_hours))| TrayCapsule {
                palette: palette(index),
                percent: *percent,
                time_hours: *time_hours,
            })
            .collect()
    }

    fn uniform(count: usize, percent: Option<u8>, time_hours: Option<u8>) -> Vec<TrayCapsule> {
        capsules(&vec![(percent, time_hours); count])
    }

    fn icon(values: &[(Option<u8>, Option<u8>)]) -> Vec<u8> {
        render_tray_capsules(&capsules(values))
    }

    /// Half-open pixel range covered by capsule `index`.
    fn capsule_bounds(index: usize, count: usize) -> (u32, u32) {
        let left = tray_capsule_left(index, count);
        (left as u32, (left + TRAY_CAPSULE_WIDTH).ceil() as u32)
    }

    /// Midpoint of the transparent gap between capsule `index` and the next one.
    fn gap_center(index: usize, count: usize) -> u32 {
        let end = tray_capsule_left(index, count) + TRAY_CAPSULE_WIDTH;
        let next = tray_capsule_left(index + 1, count);
        ((end + next) * 0.5).round() as u32
    }

    fn count_provider_pixels(rgba: &[u8], blue: bool) -> usize {
        rgba.chunks_exact(4)
            .filter(|pixel| {
                pixel[3] > 120
                    && if blue {
                        pixel[2] > pixel[0].saturating_add(20)
                    } else {
                        pixel[0] > pixel[2].saturating_add(20)
                    }
            })
            .count()
    }

    fn count_filled_pixels(rgba: &[u8], blue: bool) -> usize {
        rgba.chunks_exact(4)
            .filter(|pixel| {
                pixel[3] > 180
                    && if blue {
                        pixel[2] > 180 && pixel[1] > 70
                    } else {
                        pixel[0] > 140 && pixel[1] > 55 && pixel[2] < 110
                    }
            })
            .count()
    }

    fn count_text_pixels(rgba: &[u8], width: u32, left: u32, right: u32) -> usize {
        rgba.chunks_exact(4)
            .enumerate()
            .filter(|(index, pixel)| {
                let x = *index as u32 % width;
                let y = *index as u32 / width;
                x >= left + 10
                    && x < right.saturating_sub(10)
                    && (7..30).contains(&y)
                    && pixel[0] > 220
                    && pixel[1] > 220
                    && pixel[2] > 220
                    && pixel[3] > 150
            })
            .count()
    }

    fn region_pixels(rgba: &[u8], width: u32, left: u32, right: u32) -> Vec<[u8; 4]> {
        rgba.chunks_exact(4)
            .enumerate()
            .filter_map(|(index, pixel)| {
                let x = index as u32 % width;
                (x >= left && x < right).then(|| [pixel[0], pixel[1], pixel[2], pixel[3]])
            })
            .collect()
    }

    fn successful_snapshot(provider: &str, percent: f64) -> ProviderSnapshot {
        ProviderSnapshot {
            provider: provider.into(),
            display_name: provider.to_ascii_uppercase(),
            plan: Some("PRO".into()),
            short_window: Some(UsageWindow {
                remaining_percent: percent,
                resets_at: Some("2026-07-17T20:00:00Z".into()),
                window_seconds: 18_000,
            }),
            weekly_window: None,
            scoped_windows: Vec::new(),
            reset_credits: None,
            reset_credit_expires_at: Vec::new(),
            updated_at: "2026-07-17T18:00:00Z".into(),
            status: "ok".into(),
            message: None,
        }
    }

    /// `successful_snapshot` is stamped 2026-07-17T18:00:00Z; this is 10 minutes later.
    fn shortly_after_snapshot() -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 17, 18, 10, 0)
            .single()
            .unwrap()
    }

    #[test]
    fn transient_failure_keeps_last_good_snapshot_for_tray_and_widget() {
        let previous = successful_snapshot("codex", 74.0);
        let failure =
            ProviderSnapshot::failure_for("codex", "CODEX", "unavailable", "Network unavailable");
        let merged = merge_snapshots(
            std::slice::from_ref(&previous),
            vec![failure],
            &shortly_after_snapshot(),
        );

        assert_eq!(merged[0].status, "stale");
        assert_eq!(
            merged[0].short_window.as_ref().unwrap().remaining_percent,
            74.0
        );
        assert_eq!(merged[0].updated_at, previous.updated_at);
        assert_eq!(merged[0].message.as_deref(), Some("Network unavailable"));
    }

    #[test]
    fn stale_snapshot_is_dropped_once_it_passes_the_display_window() {
        let previous = successful_snapshot("codex", 74.0);
        let failure =
            ProviderSnapshot::failure_for("codex", "CODEX", "unavailable", "Network unavailable");
        // 25 hours after the last good reading, past the 24 hour cutoff the widget uses.
        let now = Utc
            .with_ymd_and_hms(2026, 7, 18, 19, 0, 0)
            .single()
            .unwrap();
        let merged = merge_snapshots(std::slice::from_ref(&previous), vec![failure], &now);

        assert_eq!(merged[0].status, "unavailable");
        assert!(merged[0].short_window.is_none());
    }

    /// A lapsed login is a failure like any other. Kimi's access token lives fifteen minutes and
    /// reports `signed_out` the moment it lapses, so clearing the values on that status emptied
    /// the capsule every quarter hour; the reading is dimmed to `stale` and kept instead.
    #[test]
    fn a_lapsed_login_keeps_the_last_good_values_as_stale() {
        let previous = successful_snapshot("codex", 74.0);
        let signed_out =
            ProviderSnapshot::failure_for("codex", "CODEX", "signed_out", "Please sign in");
        let merged = merge_snapshots(&[previous], vec![signed_out], &shortly_after_snapshot());

        assert_eq!(merged[0].status, "stale");
        assert_eq!(
            merged[0].short_window.as_ref().unwrap().remaining_percent,
            74.0
        );
        // The reason for the failure still reaches the menu line.
        assert_eq!(merged[0].message.as_deref(), Some("Please sign in"));
    }

    /// The four ways a fetch is known to fail — an expired token, a 401, a gateway error and a
    /// blown time budget — must all degrade the same way, because the user cannot tell them apart
    /// and none of them means the quota is unknowable.
    #[test]
    fn every_kind_of_failure_keeps_the_reading_the_same_way() {
        let previous = successful_snapshot("codex", 74.0);
        let failures = [
            ProviderSnapshot::failure_for("codex", "CODEX", "signed_out", "401"),
            ProviderSnapshot::failure_for("codex", "CODEX", "unavailable", "token expired"),
            ProviderSnapshot::failure_for("codex", "CODEX", "unavailable", "504 from the gateway"),
            timed_out_snapshot(&providers::codex::DESCRIPTOR),
        ];
        for failure in failures {
            let merged = merge_snapshots(
                std::slice::from_ref(&previous),
                vec![failure],
                &shortly_after_snapshot(),
            );
            assert_eq!(merged[0].status, "stale");
            assert_eq!(
                merged[0].short_window.as_ref().unwrap().remaining_percent,
                74.0
            );
        }
    }

    /// The other half of the rule: with nothing to fall back on, the failure is reported as-is
    /// rather than inventing a reading.
    #[test]
    fn a_failure_with_no_earlier_reading_stays_a_failure() {
        for status in ["signed_out", "unavailable"] {
            let failure = ProviderSnapshot::failure_for("codex", "CODEX", status, "no reading yet");
            let merged = merge_snapshots(&[], vec![failure], &shortly_after_snapshot());

            assert_eq!(merged[0].status, status);
            assert!(merged[0].short_window.is_none());
            assert!(merged[0].weekly_window.is_none());
        }
    }

    /// The contract's §1.5 formula, checked at the count the app shipped with: the icon must stay
    /// 172px wide with capsules at x=2 and x=90, which is what makes the bitmap byte-identical to
    /// the pre-registry build.
    #[test]
    fn two_capsules_reproduce_the_shipped_geometry() {
        assert_eq!(tray_icon_width(2), HISTORICAL_WIDTH);
        for (index, expected) in HISTORICAL_LEFTS.iter().enumerate() {
            assert_eq!(tray_capsule_left(index, 2), *expected);
        }
    }

    #[test]
    fn icon_width_follows_the_capsule_count() {
        for count in CAPSULE_COUNTS {
            let expected = count as f32 * TRAY_CAPSULE_WIDTH + (count as f32 - 1.0) * 12.0;
            assert_eq!(tray_icon_width(count), expected.round() as u32);
            // Capsules stay inside the canvas and never overlap.
            let (_, first_right) = capsule_bounds(0, count);
            assert!(first_right <= tray_icon_width(count));
            for index in 1..count {
                let (left, _) = capsule_bounds(index, count);
                let (_, previous_right) = capsule_bounds(index - 1, count);
                assert!(left >= previous_right, "capsules overlap at count {count}");
            }
            let (_, last_right) = capsule_bounds(count - 1, count);
            assert!(
                last_right <= tray_icon_width(count),
                "capsule overflows at count {count}"
            );
        }
    }

    #[test]
    fn tray_icon_has_expected_rgba_dimensions() {
        for count in CAPSULE_COUNTS {
            let rgba = render_tray_capsules(&uniform(count, Some(74), Some(3)));
            assert_eq!(
                rgba.len(),
                (tray_icon_width(count) * TRAY_ICON_HEIGHT * 4) as usize
            );
        }
    }

    #[test]
    fn provider_fill_pixels_increase_with_quota_level() {
        for count in CAPSULE_COUNTS {
            let empty = render_tray_capsules(&uniform(count, Some(0), Some(50)));
            let full = render_tray_capsules(&uniform(count, Some(100), Some(50)));
            assert!(count_filled_pixels(&full, true) > count_filled_pixels(&empty, true) + 900);
            if count > 1 {
                assert!(
                    count_filled_pixels(&full, false) > count_filled_pixels(&empty, false) + 900
                );
            }
        }
    }

    #[test]
    fn unknown_provider_keeps_color_identity_and_draws_placeholder() {
        for count in CAPSULE_COUNTS {
            let unknown = render_tray_capsules(&uniform(count, None, None));
            let width = tray_icon_width(count);
            assert!(count_provider_pixels(&unknown, true) > 300);
            if count > 1 {
                assert!(count_provider_pixels(&unknown, false) > 300);
            }
            for index in 0..count {
                let (left, right) = capsule_bounds(index, count);
                assert!(
                    count_text_pixels(&unknown, width, left, right) > 12,
                    "no placeholder in capsule {index} of {count}"
                );
            }
        }
    }

    #[test]
    fn percentage_is_rendered_inside_every_capsule() {
        for count in CAPSULE_COUNTS {
            let values: Vec<(Option<u8>, Option<u8>)> = (0..count)
                .map(|index| (Some(if index == 0 { 100 } else { 7 }), Some(50)))
                .collect();
            let rendered = icon(&values);
            let width = tray_icon_width(count);
            for index in 0..count {
                let (left, right) = capsule_bounds(index, count);
                assert!(
                    count_text_pixels(&rendered, width, left, right) > 50,
                    "no percentage in capsule {index} of {count}"
                );
            }
            let nudged: Vec<(Option<u8>, Option<u8>)> = (0..count)
                .map(|index| (Some(if index == 0 { 99 } else { 8 }), Some(50)))
                .collect();
            assert_ne!(rendered, icon(&nudged));
        }
    }

    #[test]
    fn zero_and_unknown_are_visually_distinct() {
        for count in CAPSULE_COUNTS {
            assert_ne!(
                render_tray_capsules(&uniform(count, Some(0), Some(0))),
                render_tray_capsules(&uniform(count, None, None))
            );
        }
    }

    #[test]
    fn providers_update_without_changing_the_other_capsule() {
        for count in CAPSULE_COUNTS {
            let width = tray_icon_width(count);
            let baseline_values: Vec<(Option<u8>, Option<u8>)> =
                (0..count).map(|_| (Some(24), Some(4))).collect();
            let baseline = icon(&baseline_values);
            for changed in 0..count {
                let mut values = baseline_values.clone();
                values[changed] = (Some(68), Some(4));
                let updated = icon(&values);
                for other in 0..count {
                    if other == changed {
                        continue;
                    }
                    let (left, right) = capsule_bounds(other, count);
                    assert_eq!(
                        region_pixels(&baseline, width, left, right),
                        region_pixels(&updated, width, left, right),
                        "capsule {other} moved when {changed} changed (count {count})"
                    );
                }
            }
        }
    }

    #[test]
    fn capsule_corners_and_center_gaps_remain_transparent() {
        for count in CAPSULE_COUNTS {
            let rendered = render_tray_capsules(&uniform(count, Some(100), Some(100)));
            let width = tray_icon_width(count);
            let alpha_at = |x: u32, y: u32| rendered[((y * width + x) * 4 + 3) as usize];

            assert_eq!(alpha_at(0, 0), 0, "top-left is opaque at count {count}");
            assert_eq!(
                alpha_at(width - 1, 0),
                0,
                "top-right is opaque at count {count}"
            );
            for index in 0..count.saturating_sub(1) {
                assert_eq!(
                    alpha_at(gap_center(index, count), TRAY_ICON_HEIGHT / 2),
                    0,
                    "gap {index} is opaque at count {count}"
                );
            }
        }
    }

    /// The snapshot-driven entry point must agree with the capsule-level renderer and pick each
    /// provider's own palette out of the registry.
    #[test]
    fn snapshot_entry_point_matches_the_capsule_renderer() {
        let now = shortly_after_snapshot();
        let snapshots: Vec<ProviderSnapshot> = providers::all()
            .iter()
            .map(|adapter| successful_snapshot(adapter.descriptor().id, 74.0))
            .collect();
        let (pixels, width) = tray_icon_rgba(&snapshots, &now);
        assert_eq!(width, tray_icon_width(snapshots.len()));
        assert_eq!(
            pixels,
            render_tray_capsules(&uniform(snapshots.len(), Some(74), Some(2)))
        );
    }

    /// An unregistered provider id must not be drawn rather than crash or borrow another's colour.
    #[test]
    fn snapshots_from_unknown_providers_are_skipped() {
        let now = shortly_after_snapshot();
        let (_, width) = tray_icon_rgba(&[successful_snapshot("not-a-provider", 50.0)], &now);
        assert_eq!(width, tray_icon_width(0));
    }

    #[test]
    fn placeholder_icon_has_one_capsule_per_shown_provider() {
        let (pixels, width) = empty_tray_icon_rgba();
        let count = providers::active().len();
        assert_eq!(width, tray_icon_width(count));
        assert_eq!(pixels.len(), (width * TRAY_ICON_HEIGHT * 4) as usize);
    }

    #[test]
    fn exports_preview_when_requested() {
        let Ok(path) = std::env::var("CC_TRAY_PREVIEW_PPM") else {
            return;
        };
        let count = providers::all().len();
        // Same readings the pre-registry build previewed with, so the two images stay comparable.
        let readings = [(Some(88), Some(4)), (Some(96), Some(2))];
        let values: Vec<(Option<u8>, Option<u8>)> =
            (0..count).map(|index| readings[index % 2]).collect();
        let rgba = icon(&values);
        let width = tray_icon_width(count);
        let background = [236_u8, 239_u8, 243_u8];
        let mut ppm = format!("P6\n{} {}\n255\n", width, TRAY_ICON_HEIGHT).into_bytes();
        for pixel in rgba.chunks_exact(4) {
            let alpha = pixel[3] as u16;
            for channel in 0..3 {
                let value = (pixel[channel] as u16 * alpha
                    + background[channel] as u16 * (255 - alpha)
                    + 127)
                    / 255;
                ppm.push(value as u8);
            }
        }
        std::fs::write(path, ppm).expect("failed to write requested tray preview");
    }

    #[test]
    fn reset_time_rounds_up_to_one_dot_per_remaining_hour() {
        let now = Utc
            .with_ymd_and_hms(2026, 7, 17, 10, 0, 0)
            .single()
            .unwrap();
        let window = UsageWindow {
            remaining_percent: 80.0,
            resets_at: Some("2026-07-17T12:30:00Z".into()),
            window_seconds: 18_000,
        };
        assert_eq!(time_remaining_hours(&window, &now), Some(3));

        let final_hour = UsageWindow {
            resets_at: Some("2026-07-17T10:01:00Z".into()),
            ..window.clone()
        };
        assert_eq!(time_remaining_hours(&final_hour, &now), Some(1));

        let expired = UsageWindow {
            resets_at: Some("2026-07-17T09:59:00Z".into()),
            ..window.clone()
        };
        assert_eq!(time_remaining_hours(&expired, &now), Some(0));

        let clock_skew = UsageWindow {
            resets_at: Some("2026-07-17T16:00:00Z".into()),
            ..window
        };
        assert_eq!(time_remaining_hours(&clock_skew, &now), Some(5));
    }

    #[test]
    fn hour_dots_change_with_fixed_quota_values() {
        for count in CAPSULE_COUNTS {
            let early = render_tray_capsules(&uniform(count, Some(73), Some(5)));
            let late = render_tray_capsules(&uniform(count, Some(73), Some(1)));
            assert_ne!(early, late);
        }
    }

    #[test]
    fn menu_lines_follow_the_language_preference() {
        use super::{provider_menu_line, tray_copy};

        let snapshot = successful_snapshot("codex", 74.0);
        let chinese = provider_menu_line(Some(&snapshot), "Codex", tray_copy("zh-CN"));
        let english = provider_menu_line(Some(&snapshot), "Codex", tray_copy("en"));

        assert!(chinese.contains("重置"), "unexpected: {chinese}");
        assert!(english.contains("resets"), "unexpected: {english}");
        assert!(!english.contains("重置"), "leaked Chinese: {english}");
        // Percentages are language independent, so both must still carry the reading itself.
        assert!(chinese.contains("74%") && english.contains("74%"));
    }

    #[test]
    fn placeholder_and_stale_markers_are_translated() {
        use super::{provider_menu_line, tray_copy};

        assert_eq!(
            provider_menu_line(None, "Codex", tray_copy("en")),
            "Codex · Reading…"
        );
        assert_eq!(
            provider_menu_line(None, "Codex", tray_copy("zh-CN")),
            "Codex · 正在读取…"
        );

        let stale = ProviderSnapshot {
            status: "stale".into(),
            ..successful_snapshot("codex", 74.0)
        };
        assert!(provider_menu_line(Some(&stale), "Codex", tray_copy("en")).ends_with(" · Stale"));
        assert!(
            provider_menu_line(Some(&stale), "Codex", tray_copy("zh-CN")).ends_with(" · 旧数据")
        );
    }

    #[test]
    fn reset_label_falls_back_to_the_localized_unknown_text() {
        use super::{reset_label, tray_copy};

        assert_eq!(reset_label(None, tray_copy("en")), "Reset time unknown");
        assert_eq!(reset_label(None, tray_copy("zh-CN")), "重置时间未知");
        // A malformed timestamp must degrade to the same text rather than surfacing raw input.
        assert_eq!(
            reset_label(Some("not-a-date"), tray_copy("en")),
            "Reset time unknown"
        );
    }

    /// Every provider the app is showing must be reachable from the failure path, or a signed-out
    /// account would simply vanish from the menu instead of explaining itself.
    #[test]
    fn unavailable_snapshots_cover_every_shown_provider() {
        let values = super::unavailable_snapshots("offline");
        let shown = providers::active();
        assert_eq!(values.len(), shown.len());
        for (snapshot, adapter) in values.iter().zip(shown) {
            assert_eq!(snapshot.provider, adapter.descriptor().id);
            assert_eq!(snapshot.status, "unavailable");
        }
    }
}
