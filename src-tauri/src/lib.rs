mod claude;
mod codex;
mod models;
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
const TRAY_ICON_WIDTH: u32 = 172;
const TRAY_ICON_HEIGHT: u32 = 36;
const TRAY_CAPSULE_WIDTH: f32 = 80.0;
const TRAY_CAPSULE_HEIGHT: f32 = 34.0;
const TRAY_FIRST_CAPSULE_LEFT: f32 = 2.0;
const TRAY_SECOND_CAPSULE_LEFT: f32 = 90.0;
#[cfg(test)]
const TRAY_PROVIDER_SPLIT: u32 = 86;
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

fn classify_frontmost_application(
    bundle_id: Option<&str>,
    name: Option<&str>,
) -> Option<&'static str> {
    let bundle_id = bundle_id.unwrap_or_default().to_ascii_lowercase();
    let name = name.unwrap_or_default().to_ascii_lowercase();
    // The bundle id is the reliable check; the name is a fallback for when AppKit reports no
    // identifier. It must track the product name, or focusing the app would count as "some other
    // app" and flip the orb to Claude — the flicker this guard exists to prevent.
    if bundle_id == "app.ccquota.desktop" || name == "cc" || name == "cc quota" {
        return None;
    }
    if bundle_id == "com.openai.codex" || bundle_id.contains("codex") || name.contains("codex") {
        return Some("codex");
    }
    Some("claude")
}

#[tauri::command]
fn get_frontmost_provider() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::NSWorkspace;

        let application = NSWorkspace::sharedWorkspace().frontmostApplication()?;
        let bundle_id = application
            .bundleIdentifier()
            .map(|value| value.to_string());
        let name = application.localizedName().map(|value| value.to_string());
        classify_frontmost_application(bundle_id.as_deref(), name.as_deref()).map(str::to_owned)
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

fn blend_pixel(buffer: &mut [u8], x: u32, y: u32, color: [u8; 4], coverage: f32) {
    let index = ((y * TRAY_ICON_WIDTH + x) * 4) as usize;
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

fn draw_rounded_rect(
    buffer: &mut [u8],
    left: f32,
    top: f32,
    width: f32,
    height: f32,
    radius: f32,
    color: [u8; 4],
) {
    for y in top.floor().max(0.0) as u32..(top + height).ceil().min(TRAY_ICON_HEIGHT as f32) as u32
    {
        for x in
            left.floor().max(0.0) as u32..(left + width).ceil().min(TRAY_ICON_WIDTH as f32) as u32
        {
            let amount = coverage(x, y, left, top, width, height, radius);
            if amount > 0.0 {
                blend_pixel(buffer, x, y, color, amount);
            }
        }
    }
}

fn draw_circle(buffer: &mut [u8], center_x: f32, center_y: f32, radius: f32, color: [u8; 4]) {
    let left = (center_x - radius).floor().max(0.0) as u32;
    let right = (center_x + radius).ceil().min(TRAY_ICON_WIDTH as f32) as u32;
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
                blend_pixel(buffer, x, y, color, hits as f32 / 16.0);
            }
        }
    }
}

fn draw_line(
    buffer: &mut [u8],
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
        .min(TRAY_ICON_WIDTH as f32) as u32;
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
                blend_pixel(buffer, x, y, color, hits as f32 / 16.0);
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

#[derive(Clone, Copy)]
struct CapsulePalette {
    border: [u8; 4],
    track: [u8; 4],
    fill_top: [u8; 4],
    fill_bottom: [u8; 4],
}

fn draw_capsule_fill(
    buffer: &mut [u8],
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
        for x in left.floor().max(0.0) as u32..fill_right.ceil().min(TRAY_ICON_WIDTH as f32) as u32
        {
            let amount = coverage(x, y, left, top, width, height, radius);
            if amount > 0.0 {
                let horizontal_coverage = (fill_right - x as f32).clamp(0.0, 1.0);
                blend_pixel(buffer, x, y, color, amount * horizontal_coverage);
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

fn draw_digit(buffer: &mut [u8], left: f32, top: f32, digit: u8, color: [u8; 4]) {
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

fn draw_percent_symbol(buffer: &mut [u8], left: f32, top: f32, color: [u8; 4]) {
    draw_circle(buffer, left + 2.0, top + 3.4, 1.85, color);
    draw_line(
        buffer,
        left + 8.6,
        top + 1.7,
        left + 1.4,
        top + 20.1,
        1.85,
        color,
    );
    draw_circle(buffer, left + 7.7, top + 18.4, 1.85, color);
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

fn draw_system_percent_label(buffer: &mut [u8], capsule_left: f32, percent: Option<u8>) -> bool {
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
                if x >= 0 && x < TRAY_ICON_WIDTH as i32 && y >= 0 && y < TRAY_ICON_HEIGHT as i32 {
                    blend_pixel(buffer, x as u32, y as u32, text_color, alpha);
                    if x + 1 < TRAY_ICON_WIDTH as i32 {
                        blend_pixel(buffer, (x + 1) as u32, y as u32, text_color, alpha * 0.22);
                    }
                }
            }
        }
    }
    true
}

fn draw_percent_label(buffer: &mut [u8], capsule_left: f32, percent: Option<u8>) {
    if draw_system_percent_label(buffer, capsule_left, percent) {
        return;
    }
    let text_color = [255, 255, 255, 244];
    let top = 7.1;
    let Some(percent) = percent else {
        let dash_width = 12.0;
        let left = capsule_left + (TRAY_CAPSULE_WIDTH - dash_width) * 0.5;
        draw_rounded_rect(buffer, left, 16.9, dash_width, 2.15, 1.08, text_color);
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
            left,
            top,
            character.saturating_sub(b'0'),
            text_color,
        );
        left += digit_width + digit_gap;
    }
    draw_percent_symbol(buffer, left - digit_gap + suffix_gap, top, text_color);
}

fn draw_capsule_time_dots(buffer: &mut [u8], left: f32, hours: u8) {
    let spacing = 6.0;
    let start_x = left + TRAY_CAPSULE_WIDTH * 0.5
        - spacing * (TRAY_TIME_DOT_COUNT.saturating_sub(1) as f32) * 0.5;
    let center_y = 32.0;
    let lit_hours = hours.min(TRAY_TIME_DOT_COUNT);

    for index in 0..TRAY_TIME_DOT_COUNT {
        let center_x = start_x + index as f32 * spacing;
        draw_circle(buffer, center_x, center_y, 1.25, [255, 255, 255, 48]);
        if index < lit_hours {
            draw_circle(buffer, center_x, center_y, 1.25, [255, 255, 255, 236]);
        }
    }
}

fn draw_tray_capsule(
    buffer: &mut [u8],
    left: f32,
    percent: Option<u8>,
    time_hours: Option<u8>,
    palette: CapsulePalette,
) {
    let top = 1.0;
    let radius = 17.0;
    draw_rounded_rect(
        buffer,
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
    draw_percent_label(buffer, left, percent);
    if let Some(value) = time_hours {
        draw_capsule_time_dots(buffer, left, value);
    }
}

fn tray_icon_rgba(
    codex_percent: Option<u8>,
    claude_percent: Option<u8>,
    codex_time_hours: Option<u8>,
    claude_time_hours: Option<u8>,
) -> Vec<u8> {
    let mut rgba = vec![0; (TRAY_ICON_WIDTH * TRAY_ICON_HEIGHT * 4) as usize];
    draw_tray_capsule(
        &mut rgba,
        TRAY_FIRST_CAPSULE_LEFT,
        codex_percent,
        codex_time_hours,
        CapsulePalette {
            border: [25, 55, 82, 255],
            track: [25, 55, 82, 255],
            fill_top: [47, 111, 237, 255],
            fill_bottom: [47, 111, 237, 255],
        },
    );
    draw_tray_capsule(
        &mut rgba,
        TRAY_SECOND_CAPSULE_LEFT,
        claude_percent,
        claude_time_hours,
        CapsulePalette {
            border: [91, 49, 37, 255],
            track: [91, 49, 37, 255],
            fill_top: [184, 90, 58, 255],
            fill_bottom: [184, 90, 58, 255],
        },
    );
    rgba
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
    let codex = snapshots.iter().find(|item| item.provider == "codex");
    let claude = snapshots.iter().find(|item| item.provider == "claude");
    let tooltip = format!(
        "{}\n{}",
        provider_menu_line(codex, "Codex", copy),
        provider_menu_line(claude, "Claude", copy)
    );

    let codex_detail = MenuItem::with_id(
        app,
        "codex_detail",
        provider_menu_line(codex, "Codex", copy),
        false,
        None::<&str>,
    )
    .map_err(|error| error.to_string())?;
    let claude_detail = MenuItem::with_id(
        app,
        "claude_detail",
        provider_menu_line(claude, "Claude", copy),
        false,
        None::<&str>,
    )
    .map_err(|error| error.to_string())?;
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
    let menu = Menu::with_items(
        app,
        &[
            &codex_detail,
            &claude_detail,
            &separator_top,
            &toggle_widget,
            &always_on_top,
            &unlock,
            &refresh,
            &language,
            &autostart,
            &separator_bottom,
            &quit,
        ],
    )
    .map_err(|error| error.to_string())?;

    let now = Utc::now();
    tray.set_icon_with_as_template(
        Some(Image::new_owned(
            tray_icon_rgba(
                snapshot_percent(codex),
                snapshot_percent(claude),
                snapshot_time_hours(codex, &now),
                snapshot_time_hours(claude, &now),
            ),
            TRAY_ICON_WIDTH,
            TRAY_ICON_HEIGHT,
        )),
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
    vec![
        ProviderSnapshot::failure_for("codex", "CODEX", "unavailable", message),
        ProviderSnapshot::failure_for("claude", "CLAUDE", "unavailable", message),
    ]
}

/// Mirrors `isSnapshotDisplayable` in `src/lib/format.ts`: past this age the last good reading is
/// no longer worth showing. Without it the tray keeps rendering a confident percentage from an
/// arbitrarily old fetch while the widget has already blanked out.
const MAX_STALE_SECONDS: i64 = 30 * 60;

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
            if next.status == "ok" || next.status == "signed_out" {
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

    let (codex_snapshot, claude_snapshot) = tokio::join!(
        codex::fetch_snapshot(&state.client),
        claude::fetch_snapshot(&state.client),
    );
    let current = state
        .snapshot_cache
        .lock()
        .ok()
        .and_then(|cache| cache.as_ref().map(|(_, values)| values.clone()))
        .unwrap_or_default();
    let values = merge_snapshots(&current, vec![codex_snapshot, claude_snapshot], &Utc::now());
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
    let codex_detail = MenuItem::with_id(
        app,
        "codex_detail",
        provider_menu_line(None, "Codex", copy),
        false,
        None::<&str>,
    )?;
    let claude_detail = MenuItem::with_id(
        app,
        "claude_detail",
        provider_menu_line(None, "Claude", copy),
        false,
        None::<&str>,
    )?;
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
    let menu = Menu::with_items(
        app,
        &[
            &codex_detail,
            &claude_detail,
            &separator_top,
            &toggle_widget,
            &refresh,
            &separator_bottom,
            &quit,
        ],
    )?;
    TrayIconBuilder::with_id("main")
        .icon(Image::new_owned(
            tray_icon_rgba(None, None, None, None),
            TRAY_ICON_WIDTH,
            TRAY_ICON_HEIGHT,
        ))
        .icon_as_template(false)
        .menu(&menu)
        .show_menu_on_left_click(true)
        .tooltip("CC Quota · Codex & Claude")
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
                .timeout(Duration::from_secs(12))
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
            get_frontmost_provider
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
mod tray_icon_tests {
    use super::{
        classify_frontmost_application, merge_snapshots, time_remaining_hours, tray_icon_rgba,
        ProviderSnapshot, UsageWindow, TRAY_ICON_HEIGHT, TRAY_ICON_WIDTH, TRAY_PROVIDER_SPLIT,
    };
    use chrono::{TimeZone, Utc};

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

    fn count_text_pixels(rgba: &[u8], left: u32, right: u32) -> usize {
        rgba.chunks_exact(4)
            .enumerate()
            .filter(|(index, pixel)| {
                let x = *index as u32 % TRAY_ICON_WIDTH;
                let y = *index as u32 / TRAY_ICON_WIDTH;
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

    fn region_pixels(rgba: &[u8], left: u32, right: u32) -> Vec<[u8; 4]> {
        rgba.chunks_exact(4)
            .enumerate()
            .filter_map(|(index, pixel)| {
                let x = index as u32 % TRAY_ICON_WIDTH;
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
        // 31 minutes after the last good reading, past the 30 minute cutoff the widget uses.
        let now = Utc
            .with_ymd_and_hms(2026, 7, 17, 18, 31, 0)
            .single()
            .unwrap();
        let merged = merge_snapshots(std::slice::from_ref(&previous), vec![failure], &now);

        assert_eq!(merged[0].status, "unavailable");
        assert!(merged[0].short_window.is_none());
    }

    #[test]
    fn signed_out_snapshot_clears_last_good_values() {
        let previous = successful_snapshot("codex", 74.0);
        let signed_out =
            ProviderSnapshot::failure_for("codex", "CODEX", "signed_out", "Please sign in");
        let merged = merge_snapshots(&[previous], vec![signed_out], &shortly_after_snapshot());

        assert_eq!(merged[0].status, "signed_out");
        assert!(merged[0].short_window.is_none());
    }

    #[test]
    fn tray_icon_has_expected_rgba_dimensions() {
        let rgba = tray_icon_rgba(Some(74), Some(58), Some(62), Some(41));
        assert_eq!(
            rgba.len(),
            (TRAY_ICON_WIDTH * TRAY_ICON_HEIGHT * 4) as usize
        );
    }

    #[test]
    fn provider_fill_pixels_increase_with_quota_level() {
        let empty = tray_icon_rgba(Some(0), Some(0), Some(50), Some(50));
        let full = tray_icon_rgba(Some(100), Some(100), Some(50), Some(50));
        assert!(count_filled_pixels(&full, true) > count_filled_pixels(&empty, true) + 900);
        assert!(count_filled_pixels(&full, false) > count_filled_pixels(&empty, false) + 900);
    }

    #[test]
    fn unknown_provider_keeps_color_identity_and_draws_placeholder() {
        let unknown = tray_icon_rgba(None, None, None, None);
        assert!(count_provider_pixels(&unknown, true) > 300);
        assert!(count_provider_pixels(&unknown, false) > 300);
        assert!(count_text_pixels(&unknown, 0, TRAY_PROVIDER_SPLIT) > 12);
        assert!(count_text_pixels(&unknown, TRAY_PROVIDER_SPLIT, TRAY_ICON_WIDTH) > 12);
    }

    #[test]
    fn percentage_is_rendered_inside_both_capsules() {
        let icon = tray_icon_rgba(Some(100), Some(7), Some(50), Some(50));
        assert!(count_text_pixels(&icon, 0, TRAY_PROVIDER_SPLIT) > 80);
        assert!(count_text_pixels(&icon, TRAY_PROVIDER_SPLIT, TRAY_ICON_WIDTH) > 50);
        assert_ne!(icon, tray_icon_rgba(Some(99), Some(8), Some(50), Some(50)));
    }

    #[test]
    fn zero_and_unknown_are_visually_distinct() {
        assert_ne!(
            tray_icon_rgba(Some(0), Some(0), Some(0), Some(0)),
            tray_icon_rgba(None, None, None, None)
        );
    }

    #[test]
    fn providers_update_without_changing_the_other_capsule() {
        let baseline = tray_icon_rgba(Some(24), Some(81), Some(64), Some(36));
        let codex_changed = tray_icon_rgba(Some(68), Some(81), Some(64), Some(36));
        let claude_changed = tray_icon_rgba(Some(24), Some(37), Some(64), Some(36));

        assert_eq!(
            region_pixels(&baseline, TRAY_PROVIDER_SPLIT, TRAY_ICON_WIDTH),
            region_pixels(&codex_changed, TRAY_PROVIDER_SPLIT, TRAY_ICON_WIDTH)
        );
        assert_eq!(
            region_pixels(&baseline, 0, TRAY_PROVIDER_SPLIT),
            region_pixels(&claude_changed, 0, TRAY_PROVIDER_SPLIT)
        );
    }

    #[test]
    fn capsule_corners_and_center_gap_remain_transparent() {
        let icon = tray_icon_rgba(Some(100), Some(100), Some(100), Some(100));
        let alpha_at = |x: u32, y: u32| icon[((y * TRAY_ICON_WIDTH + x) * 4 + 3) as usize];

        assert_eq!(alpha_at(0, 0), 0);
        assert_eq!(alpha_at(TRAY_ICON_WIDTH - 1, 0), 0);
        assert_eq!(alpha_at(TRAY_PROVIDER_SPLIT, TRAY_ICON_HEIGHT / 2), 0);
    }

    #[test]
    fn exports_preview_when_requested() {
        let Ok(path) = std::env::var("CC_TRAY_PREVIEW_PPM") else {
            return;
        };
        let rgba = tray_icon_rgba(Some(88), Some(96), Some(4), Some(2));
        let background = [236_u8, 239_u8, 243_u8];
        let mut ppm = format!("P6\n{} {}\n255\n", TRAY_ICON_WIDTH, TRAY_ICON_HEIGHT).into_bytes();
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
        let early = tray_icon_rgba(Some(73), Some(64), Some(5), Some(4));
        let late = tray_icon_rgba(Some(73), Some(64), Some(1), Some(1));
        assert_ne!(early, late);
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

    #[test]
    fn maps_codex_bundle_to_codex_and_every_other_app_to_claude() {
        assert_eq!(
            classify_frontmost_application(Some("com.openai.codex"), Some("ChatGPT")),
            Some("codex")
        );
        assert_eq!(
            classify_frontmost_application(Some("com.anthropic.claudefordesktop"), Some("Claude")),
            Some("claude")
        );
        assert_eq!(
            classify_frontmost_application(Some("com.apple.finder"), Some("Finder")),
            Some("claude")
        );
    }

    #[test]
    fn ignores_cc_itself_to_avoid_focus_flicker() {
        assert_eq!(
            classify_frontmost_application(Some("app.ccquota.desktop"), Some("CC Quota")),
            None
        );
        // Falls back to the product name when AppKit reports no bundle identifier.
        assert_eq!(classify_frontmost_application(None, Some("CC Quota")), None);
        assert_eq!(classify_frontmost_application(None, Some("CC")), None);
    }
}
