//! The handheld: the fleet in your pocket.
//!
//! Two jobs, deliberately separable. *Viewing* puts the camera behind a
//! machine's eyes while it carries on working — you are a passenger. *Control*
//! is the master override: the machine drops what it was doing and answers to
//! the sticks. Keeping them apart means you can watch a drone work without
//! hijacking it, which is most of what you actually want to do.
//!
//! The panel itself is pixels, exactly like the shop and the HUD: a plain RGBA
//! buffer stamped with the bitmap font and shipped through an overlay slot.
//! The compositor knows nothing about what any of it means.

use glam::{Mat4, Vec3};
use vx_render::font::{self, LINE_HEIGHT};
use vx_render::OverlayRect;
use vx_world::World;

use crate::map::{self, MapState, Marker};
use crate::mining::{MachineListing, MachineRef};

/// Panel size in texture pixels.
///
/// No scale constant any more: the readout is not drawn at a size somebody
/// chose, it is drawn *on the glass of the unit you are holding*, and how big
/// that lands is a question for the camera — see [`screen_rect`].
pub const DEVICE_WIDTH: u32 = 240;
pub const DEVICE_HEIGHT: u32 = 166;

/// The strip along the top of a live feed.
pub const BANNER_WIDTH: u32 = 240;
pub const BANNER_HEIGHT: u32 = 30;
pub const BANNER_SCALE: f32 = 2.0;

/// Distance at which the signal bar reads empty. Cosmetic, but honest: it is
/// a real measure of how far you have wandered from your own body.
pub const SIGNAL_RANGE: f32 = 320.0;

const TEXT: [u8; 4] = [235, 235, 235, 255];
const DIM: [u8; 4] = [150, 150, 155, 255];
const ACCENT: [u8; 4] = [255, 170, 60, 255];
const LIVE: [u8; 4] = [120, 220, 120, 255];
const BAR_BACK: [u8; 4] = [45, 48, 55, 255];
const BACKGROUND: [u8; 4] = [10, 12, 16, 235];
const BANNER_BACK: [u8; 4] = [10, 12, 16, 170];

/// Side of the handheld's map, in panel pixels.
pub const HANDHELD_MAP: u32 = 120;

// ----------------------------------------------------------- holding it up
//
// The handheld is a *thing you hold*, and the geometry of holding it lives
// here rather than in `main` for the reason every panel's layout does: it is
// pure arithmetic over numbers, so it can be tested without a window, a
// world, or a graphics device anywhere near it.

/// Seconds from stowed to raised, and back.
///
/// Long enough to read as a movement, short enough that nobody waits on it.
pub const RAISE_SECONDS: f32 = 0.32;

/// Where the unit rides when it is down: below the frame, out of the way,
/// tipped away from the face. Camera-relative, in (forward, right, up).
const STOWED: (f32, f32, f32) = (0.62, 0.30, -0.75);

/// And where it comes to rest, in front of the face and a little to the
/// right, tipped back so the glass looks at you.
///
/// The forward distance is the number that decides how big the screen lands
/// on a monitor, and it is picked so the projected rectangle is close to the
/// size the flat panel used to be drawn at — the overlay samples with
/// `Nearest`, so a screen much smaller than its own pixels is a screen you
/// cannot read.
const RAISED: (f32, f32, f32) = (0.60, 0.07, -0.15);

/// How far the case is tipped, in radians, stowed and raised. Positive tips
/// the top of the screen away.
const STOWED_TILT: f32 = 0.85;
const RAISED_TILT: f32 = 0.14;

/// Smooth the ends of the raise, so it does not start and stop dead.
fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Where the unit sits this frame, and how far it is tipped.
///
/// Returned in camera-relative components rather than as a world point, so
/// the caller composes it with whatever basis it already has and there is one
/// place that decides *shape of the motion* rather than two.
pub fn carry(raise: f32) -> (f32, f32, f32, f32) {
    let t = ease(raise);
    let lerp = |from: f32, to: f32| from + (to - from) * t;
    (
        lerp(STOWED.0, RAISED.0),
        lerp(STOWED.1, RAISED.1),
        lerp(STOWED.2, RAISED.2),
        lerp(STOWED_TILT, RAISED_TILT),
    )
}

/// Where the unit's body sits in the world, given the camera's basis.
pub fn carried_at(eye: Vec3, forward: Vec3, right: Vec3, raise: f32) -> Vec3 {
    let (ahead, across, down, _) = carry(raise);
    eye + forward * ahead + right * across + Vec3::Y * down
}

/// The four corners of the glass in the world, clockwise from top-left as
/// the person holding it sees them.
///
/// The screen's own dimensions come from [`crate::rig::screen`], which is
/// also what [`crate::rig::Rig::handheld`] builds the plate from — one set of
/// numbers, so the readout cannot land somewhere the model is not.
pub fn screen_corners(eye: Vec3, forward: Vec3, right: Vec3, raise: f32) -> [Vec3; 4] {
    use crate::rig::screen;

    let (_, _, _, tilt) = carry(raise);
    let centre = carried_at(eye, forward, right, raise);

    // The rig is pitched about its lateral axis by `tilt`, so the glass's own
    // up and normal turn with it. Across is the lateral axis and does not.
    let up = Vec3::Y * tilt.cos() + forward * tilt.sin();
    let normal = forward * tilt.cos() - Vec3::Y * tilt.sin();

    // `screen::DEPTH` is negative — the glass faces back at the holder.
    let face = centre + normal * screen::DEPTH;
    let across = right * screen::HALF_WIDTH;
    let rise = up * screen::HALF_HEIGHT;
    [
        face - across + rise,
        face + across + rise,
        face + across - rise,
        face - across - rise,
    ]
}

/// The screen's four corners as a rectangle of pixels on the frame.
///
/// `None` when any corner is behind the camera: a rectangle derived from a
/// point behind the eye is a rectangle in the wrong place, and drawing
/// nothing is the honest answer while the unit is still coming up from
/// somewhere off-frame.
pub fn screen_rect(
    view_projection: Mat4,
    corners: [Vec3; 4],
    size: (f32, f32),
) -> Option<OverlayRect> {
    let (width, height) = size;
    let mut min = (f32::MAX, f32::MAX);
    let mut max = (f32::MIN, f32::MIN);
    for corner in corners {
        let clip = view_projection * corner.extend(1.0);
        if clip.w <= 1.0e-4 {
            return None;
        }
        let ndc = (clip.x / clip.w, clip.y / clip.w);
        // Clip space is y-up from the centre; pixels are y-down from the
        // top-left, the same mapping the overlay shader undoes.
        let pixel = (
            (ndc.0 * 0.5 + 0.5) * width,
            (0.5 - ndc.1 * 0.5) * height,
        );
        min = (min.0.min(pixel.0), min.1.min(pixel.1));
        max = (max.0.max(pixel.0), max.1.max(pixel.1));
    }

    let (box_width, box_height) = (max.0 - min.0, max.1 - min.1);
    if box_width <= 1.0 || box_height <= 1.0 {
        return None;
    }

    // The glass is tilted toward you, so its projection is a trapezoid and
    // the box around it is wider than the face. Stretching the readout into
    // that box would squash the text by however far the unit happens to be
    // tipped, so the picture is *fitted* inside the box at its own shape
    // instead, and centred. Letterboxing a screen is what a real one does
    // with the wrong aspect anyway.
    let want = DEVICE_WIDTH as f32 / DEVICE_HEIGHT as f32;
    let (width, height) = if box_width / box_height > want {
        (box_height * want, box_height)
    } else {
        (box_width, box_width / want)
    };

    Some(OverlayRect {
        x: min.0 + (box_width - width) * 0.5,
        y: min.1 + (box_height - height) * 0.5,
        width,
        height,
    })
}

/// The readout in the middle of the frame, at twice its own pixels.
///
/// The fallback for a view with no hands in it: third person draws no
/// viewmodel, so there is no glass to land on and a screen floating in the
/// air where a unit is not would be worse than the flat panel this replaced.
pub fn centred(width: f32, height: f32) -> OverlayRect {
    let panel_width = DEVICE_WIDTH as f32 * 2.0;
    let panel_height = DEVICE_HEIGHT as f32 * 2.0;
    OverlayRect {
        x: (width - panel_width) / 2.0,
        y: (height - panel_height) / 2.0,
        width: panel_width,
        height: panel_height,
    }
}

/// Fade the readout up with the raise: a screen that is at full brightness
/// while the thing carrying it is still swinging into view reads as a
/// sticker rather than as a display coming on.
///
/// Done to the pixels rather than in the shader because the overlay pass has
/// no tint uniform, and the buffer is rebuilt every frame anyway.
pub fn dim(pixels: &mut [u8], fraction: f32) {
    let scale = ease(fraction).clamp(0.0, 1.0);
    if scale >= 1.0 {
        return;
    }
    for texel in pixels.chunks_exact_mut(4) {
        texel[3] = (texel[3] as f32 * scale) as u8;
    }
}

/// What the handheld is showing.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    /// The fleet: who is out there and what they are doing.
    #[default]
    Fleet,
    /// The country: where you have been, and everything on it.
    Map,
    /// The scout's standing orders.
    Kestrel,
    /// The toy. Nothing on this page is about the world at all, which is
    /// the point of owning it.
    Arcade,
}

/// What the kestrel page needs to know, read once per frame.
pub struct ScoutReadout {
    /// One word: DOCKED, COOLING, ORBITING...
    pub state: &'static str,
    /// Flight ticks left in the cell.
    pub endurance: u32,
    /// Recharge ticks left.
    pub cooldown: u32,
    /// The order rows in cursor order. Context-sensitive: the standing
    /// orders always, plus whatever the machine is close enough to work on.
    /// Built by the caller, because what a lock *is* is fiction the handheld
    /// has no business knowing.
    pub rows: Vec<String>,
    /// What the coil is doing right now, if anything.
    pub job: Option<String>,
}

/// The handheld's state.
#[derive(Debug, Default)]
pub struct Device {
    /// Is the roster panel up?
    pub open: bool,
    /// How far the unit has been raised, 0 stowed to 1 in front of your
    /// face. Live-only cosmetics, like every other animation clock here.
    pub raise: f32,
    /// Which page it is showing.
    pub page: Page,
    cursor: usize,
    /// The machine the camera is riding, if any.
    viewing: Option<MachineRef>,
    /// The machine actually taking orders, if any.
    piloting: Option<MachineRef>,
    /// The last action's outcome, shown until the next one.
    pub feedback: Option<String>,
}

impl Device {
    pub fn new() -> Self {
        Device::default()
    }

    pub fn open_list(&mut self) {
        self.open = true;
        self.feedback = None;
    }

    /// Run the raise toward wherever it is going. Returns whether anything
    /// of the unit is still on screen, which is what tells the caller when
    /// it is finally safe to clear the overlay.
    pub fn raise_by(&mut self, dt: f32) -> bool {
        let step = dt / RAISE_SECONDS;
        if self.open {
            self.raise = (self.raise + step).min(1.0);
        } else {
            self.raise = (self.raise - step).max(0.0);
        }
        self.raise > 0.0
    }

    /// Is the unit far enough up to be worth drawing the screen on?
    pub fn showing(&self) -> bool {
        self.raise > 0.0
    }

    /// Turn to the next page: fleet, map, kestrel, round again.
    pub fn turn_page(&mut self) {
        self.page = match self.page {
            Page::Fleet => Page::Map,
            Page::Map => Page::Kestrel,
            Page::Kestrel => Page::Arcade,
            Page::Arcade => Page::Fleet,
        };
        self.cursor = 0;
        self.feedback = None;
    }

    /// Where the cursor stands, for pages whose rows are not machines.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Close the panel. A live feed keeps running — you put the handheld down
    /// still watching, which is the point of it.
    pub fn close(&mut self) {
        self.open = false;
    }

    pub fn move_cursor(&mut self, delta: i32, rows: usize) {
        if rows == 0 {
            self.cursor = 0;
            return;
        }
        self.cursor = (self.cursor as i32 + delta).clamp(0, rows as i32 - 1) as usize;
    }

    /// The machine the cursor is on.
    pub fn selected(&self, roster: &[MachineListing]) -> Option<MachineRef> {
        roster.get(self.cursor).map(|row| row.machine)
    }

    /// Ride a machine's camera without touching its controls.
    pub fn view(&mut self, machine: MachineRef) {
        self.viewing = Some(machine);
        self.open = false;
        self.feedback = None;
    }

    /// Master override on or off for whatever is being viewed.
    ///
    /// Returns `(machine, taking)` when the caller needs to actually take or
    /// release control, so the state here and the simulation cannot disagree.
    pub fn toggle_control(&mut self) -> Option<(MachineRef, bool)> {
        match (self.piloting, self.viewing) {
            (Some(machine), _) => {
                self.piloting = None;
                Some((machine, false))
            }
            (None, Some(machine)) => {
                self.piloting = Some(machine);
                self.open = false;
                Some((machine, true))
            }
            (None, None) => None,
        }
    }

    /// Whichever machine the camera should sit on.
    pub fn feed(&self) -> Option<MachineRef> {
        self.viewing
    }

    pub fn is_piloting(&self) -> bool {
        self.piloting.is_some()
    }

    /// Hang up. Returns the machine that needs handing back, if one was being
    /// driven.
    pub fn hand_back(&mut self) -> Option<MachineRef> {
        self.viewing = None;
        self.open = false;
        self.feedback = None;
        self.piloting.take()
    }
}

/// Signal strength from the pilot's distance to the machine, 0..1.
pub fn signal(distance: f32) -> f32 {
    (1.0 - distance / SIGNAL_RANGE).clamp(0.0, 1.0)
}

/// Paint a horizontal fill bar. Takes its rectangle as one value rather than
/// four loose numbers, which is also harder to pass in the wrong order.
fn draw_bar(
    pixels: &mut [u8],
    stride: u32,
    rect: (u32, u32, u32, u32),
    fraction: f32,
    colour: [u8; 4],
) {
    let (x, y, width, height) = rect;
    let filled = (width as f32 * fraction.clamp(0.0, 1.0)) as u32;
    for py in y..y + height {
        for px in x..x + width {
            let at = ((py * stride + px) * 4) as usize;
            if at + 4 > pixels.len() {
                continue;
            }
            let texel = if px < x + filled { colour } else { BAR_BACK };
            pixels[at..at + 4].copy_from_slice(&texel);
        }
    }
}

/// Draw the roster panel.
/// What the handheld's map draws over.
pub struct Country<'a> {
    pub world: &'a World,
    pub explored: &'a MapState,
    pub centre: (i32, i32),
    pub markers: &'a [Marker],
}

pub fn render_device(
    device: &Device,
    roster: &[MachineListing],
    country: Option<&Country<'_>>,
    scout: Option<&ScoutReadout>,
) -> Vec<u8> {
    let mut pixels = vec![0u8; (DEVICE_WIDTH * DEVICE_HEIGHT * 4) as usize];
    for texel in pixels.chunks_exact_mut(4) {
        texel.copy_from_slice(&BACKGROUND);
    }

    let margin = 6i32;
    let mut y = margin;

    if device.page == Page::Map {
        font::draw_text(&mut pixels, DEVICE_WIDTH, margin, y, 1, ACCENT, "COUNTRY");
        y += LINE_HEIGHT as i32 + 2;
        if let Some(country) = country {
            // The same picture the corner minimap draws, at the size a
            // handheld can hold: your machines, the towns you have found, and
            // the black you have not walked into yet.
            let inset = map::render_map_sized(
                country.world,
                country.explored,
                country.centre,
                2,
                HANDHELD_MAP,
                country.markers,
            );
            let left = (DEVICE_WIDTH as i32 - HANDHELD_MAP as i32) / 2;
            map::blit(&mut pixels, DEVICE_WIDTH, &inset, HANDHELD_MAP, left, y);
            map::frame(&mut pixels, DEVICE_WIDTH, HANDHELD_MAP, left, y, DIM);
            y += HANDHELD_MAP as i32 + 3;
            let here = format!("{} {}", country.centre.0, country.centre.1);
            font::draw_text(&mut pixels, DEVICE_WIDTH, margin, y, 1, DIM, &here);
        } else {
            font::draw_text(&mut pixels, DEVICE_WIDTH, margin, y, 1, DIM, "NO SIGNAL");
        }
        font::draw_text(
            &mut pixels,
            DEVICE_WIDTH,
            margin,
            DEVICE_HEIGHT as i32 - LINE_HEIGHT as i32 - 2,
            1,
            DIM,
            "TAB TURNS THE PAGE. V CLOSES.",
        );
        return pixels;
    }

    if device.page == Page::Kestrel {
        font::draw_text(&mut pixels, DEVICE_WIDTH, margin, y, 1, ACCENT, "KESTREL COMMAND");
        y += LINE_HEIGHT as i32 + 3;
        match scout {
            Some(scout) => {
                // The budget, in seconds at the 8 Hz clock: what is left to
                // fly, or how long until it can.
                let line = if scout.cooldown > 0 {
                    format!("{} - READY IN {}S", scout.state, scout.cooldown / 8)
                } else {
                    format!("{} - {}S OF FLIGHT", scout.state, scout.endurance / 8)
                };
                font::draw_text(&mut pixels, DEVICE_WIDTH, margin, y, 1, TEXT, &line);
                y += LINE_HEIGHT as i32;
                if let Some(job) = &scout.job {
                    font::draw_text(&mut pixels, DEVICE_WIDTH, margin, y, 1, LIVE, job);
                    y += LINE_HEIGHT as i32;
                }
                y += 3;
                for (index, order) in scout.rows.iter().enumerate() {
                    let selected = index == device.cursor;
                    if selected {
                        font::draw_text(&mut pixels, DEVICE_WIDTH, margin, y, 1, ACCENT, ">");
                    }
                    let colour = if selected { TEXT } else { DIM };
                    font::draw_text(&mut pixels, DEVICE_WIDTH, margin + 10, y, 1, colour, order);
                    y += LINE_HEIGHT as i32;
                }
            }
            None => {
                font::draw_text(
                    &mut pixels,
                    DEVICE_WIDTH,
                    margin,
                    y,
                    1,
                    DIM,
                    "NO KESTREL. THE SHOP COUNTER SELLS THEM",
                );
            }
        }
        if let Some(feedback) = &device.feedback {
            y += 3;
            font::draw_text(&mut pixels, DEVICE_WIDTH, margin, y, 1, LIVE, feedback);
        }
        font::draw_text(
            &mut pixels,
            DEVICE_WIDTH,
            margin,
            DEVICE_HEIGHT as i32 - LINE_HEIGHT as i32 - 2,
            1,
            DIM,
            "ENTER ORDERS. TAB TURNS THE PAGE. V CLOSES.",
        );
        return pixels;
    }

    font::draw_text(&mut pixels, DEVICE_WIDTH, margin, y, 1, ACCENT, "FLEET UPLINK");
    y += LINE_HEIGHT as i32 + 3;

    if roster.is_empty() {
        font::draw_text(&mut pixels, DEVICE_WIDTH, margin, y, 1, DIM, "NO MACHINES IN RANGE");
    }

    for (index, row) in roster.iter().enumerate() {
        let selected = index == device.cursor;
        if selected {
            font::draw_text(&mut pixels, DEVICE_WIDTH, margin, y, 1, ACCENT, ">");
        }
        let colour = if selected { TEXT } else { DIM };
        let line = format!("{} {}", row.name, row.state);
        font::draw_text(&mut pixels, DEVICE_WIDTH, margin + 10, y, 1, colour, &line);
        let detail = format!("{:.0}M {}/{}", row.distance, row.cargo, row.capacity);
        let at = DEVICE_WIDTH as i32 - margin - font::text_width(&detail, 1) as i32;
        font::draw_text(&mut pixels, DEVICE_WIDTH, at, y, 1, DIM, &detail);
        y += LINE_HEIGHT as i32;
    }

    if let Some(feedback) = &device.feedback {
        y += 3;
        font::draw_text(&mut pixels, DEVICE_WIDTH, margin, y, 1, LIVE, feedback);
    }

    font::draw_text(
        &mut pixels,
        DEVICE_WIDTH,
        margin,
        DEVICE_HEIGHT as i32 - LINE_HEIGHT as i32 - 2,
        1,
        DIM,
        "ENTER VIEWS. R TAKES OVER. V CLOSES.",
    );
    pixels
}

/// Draw the banner that sits over a live feed.
pub fn render_feed_banner(listing: &MachineListing, piloting: bool, signal: f32) -> Vec<u8> {
    let mut pixels = vec![0u8; (BANNER_WIDTH * BANNER_HEIGHT * 4) as usize];
    for texel in pixels.chunks_exact_mut(4) {
        texel.copy_from_slice(&BANNER_BACK);
    }

    let margin = 5i32;
    let heading = if piloting {
        format!("PILOTING {}", listing.name)
    } else {
        format!("FEED {}", listing.name)
    };
    let colour = if piloting { ACCENT } else { LIVE };
    font::draw_text(&mut pixels, BANNER_WIDTH, margin, margin, 1, colour, &heading);

    let detail = format!("{} CARGO {}/{}", listing.state, listing.cargo, listing.capacity);
    font::draw_text(
        &mut pixels,
        BANNER_WIDTH,
        margin,
        margin + LINE_HEIGHT as i32,
        1,
        TEXT,
        &detail,
    );

    // Signal strength: how far you have wandered from your own body.
    let label_width = font::text_width("SIG", 1) as i32;
    let bar_x = BANNER_WIDTH as i32 - margin - 60;
    font::draw_text(
        &mut pixels,
        BANNER_WIDTH,
        bar_x - label_width - 4,
        margin,
        1,
        DIM,
        "SIG",
    );
    draw_bar(
        &mut pixels,
        BANNER_WIDTH,
        (bar_x as u32, margin as u32 + 1, 60, 5),
        signal,
        if signal > 0.25 { LIVE } else { ACCENT },
    );
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roster() -> Vec<MachineListing> {
        vec![
            MachineListing {
                damage: crate::integrity::Damage::Sound,
                condition: crate::wear::Condition::Fresh,
                machine: MachineRef::Digger(0),
                name: "DIGGER 1".into(),
                state: "DIGGING".into(),
                distance: 42.0,
                cargo: 18,
                capacity: 64,
            },
            MachineListing {
                damage: crate::integrity::Damage::Sound,
                condition: crate::wear::Condition::Fresh,
                machine: MachineRef::Flier(0),
                name: "FLIER 1".into(),
                state: "SCANNING".into(),
                distance: 96.0,
                cargo: 0,
                capacity: 96,
            },
        ]
    }

    #[test]
    fn the_cursor_clamps_to_the_roster() {
        let mut device = Device::new();
        device.move_cursor(9, 2);
        assert_eq!(device.cursor, 1);
        device.move_cursor(-9, 2);
        assert_eq!(device.cursor, 0);
        // An empty roster must not index anything.
        device.move_cursor(3, 0);
        assert_eq!(device.cursor, 0);
        assert_eq!(device.selected(&[]), None);
    }

    #[test]
    fn viewing_does_not_take_control_but_the_override_does() {
        let mut device = Device::new();
        let roster = roster();
        device.open_list();
        device.move_cursor(1, roster.len());
        let machine = device.selected(&roster).unwrap();
        assert_eq!(machine, MachineRef::Flier(0));

        device.view(machine);
        assert_eq!(device.feed(), Some(machine));
        assert!(!device.is_piloting(), "merely watching took the wheel");
        assert!(!device.open, "viewing should put the panel away");

        assert_eq!(device.toggle_control(), Some((machine, true)));
        assert!(device.is_piloting());
        assert_eq!(device.feed(), Some(machine), "control lost the feed");

        // And back off again, still watching.
        assert_eq!(device.toggle_control(), Some((machine, false)));
        assert!(!device.is_piloting());
        assert_eq!(device.feed(), Some(machine));
    }

    #[test]
    fn the_override_does_nothing_with_no_feed_to_take_over() {
        let mut device = Device::new();
        device.open_list();
        assert_eq!(device.toggle_control(), None);
        assert!(!device.is_piloting());
    }

    #[test]
    fn hand_back_clears_both_the_feed_and_the_control() {
        let mut device = Device::new();
        let machine = MachineRef::Digger(0);
        device.view(machine);
        device.toggle_control();

        assert_eq!(device.hand_back(), Some(machine), "the caller was not told to release");
        assert_eq!(device.feed(), None);
        assert!(!device.is_piloting());
        assert!(!device.open);

        // Hanging up twice is harmless and asks for no second release.
        assert_eq!(device.hand_back(), None);
    }

    #[test]
    fn the_signal_falls_off_with_distance() {
        assert_eq!(signal(0.0), 1.0);
        assert!(signal(SIGNAL_RANGE * 0.5) < 1.0);
        assert_eq!(signal(SIGNAL_RANGE * 2.0), 0.0, "signal went negative");
    }

    #[test]
    fn the_panel_renders_at_its_declared_size_and_is_deterministic() {
        let mut device = Device::new();
        device.open_list();
        let roster = roster();

        let a = render_device(&device, &roster, None, None);
        let b = render_device(&device, &roster, None, None);
        assert_eq!(a.len(), (DEVICE_WIDTH * DEVICE_HEIGHT * 4) as usize);
        assert_eq!(a, b);

        // An empty fleet reads differently from a stocked one.
        assert_ne!(a, render_device(&device, &[], None, None));

        // And moving the cursor is visible.
        device.move_cursor(1, roster.len());
        assert_ne!(a, render_device(&device, &roster, None, None));
    }

    #[test]
    fn the_banner_says_whether_you_are_driving_or_just_watching() {
        let roster = roster();
        let watching = render_feed_banner(&roster[0], false, 1.0);
        let driving = render_feed_banner(&roster[0], true, 1.0);
        assert_eq!(watching.len(), (BANNER_WIDTH * BANNER_HEIGHT * 4) as usize);
        assert_ne!(watching, driving, "the banner does not distinguish the two");

        // And the signal bar actually moves.
        assert_ne!(driving, render_feed_banner(&roster[0], true, 0.1));
    }

    #[test]
    fn every_panel_string_is_drawable_by_the_font() {
        // The font has no '$' and no lower case; anything it cannot draw would
        // silently render as a placeholder box.
        let mut device = Device::new();
        device.open_list();
        device.feedback = Some("CONTROL TAKEN".into());
        let roster = roster();
        for text in [
            "FLEET UPLINK",
            "NO MACHINES IN RANGE",
            "ENTER VIEWS. R TAKES OVER. V CLOSES.",
            "SIG",
            "PILOTING DIGGER 1",
            "FEED DIGGER 1",
            "CONTROL TAKEN",
            &format!("{} {}", roster[0].name, roster[0].state),
            &format!("{:.0}M {}/{}", roster[0].distance, roster[0].cargo, roster[0].capacity),
        ] {
            for character in text.chars() {
                assert!(
                    font::glyph(character) != font::glyph('\u{0}') || character == ' ',
                    "the font cannot draw {character:?} in {text:?}"
                );
            }
        }
    }

    #[test]
    fn the_handheld_turns_between_the_fleet_the_country_and_the_kestrel() {
        let mut device = Device::new();
        device.open_list();
        assert_eq!(device.page, Page::Fleet);
        device.turn_page();
        assert_eq!(device.page, Page::Map);
        device.turn_page();
        assert_eq!(device.page, Page::Kestrel);
        device.turn_page();
        assert_eq!(device.page, Page::Arcade);
        device.turn_page();
        assert_eq!(device.page, Page::Fleet);
    }

    #[test]
    fn the_kestrel_page_lists_orders_or_the_shop_hint() {
        let mut device = Device::new();
        device.open_list();
        device.turn_page();
        device.turn_page();
        assert_eq!(device.page, Page::Kestrel);

        let readout = ScoutReadout {
            state: "DOCKED",
            endurance: 360,
            cooldown: 0,
            rows: vec!["ORBIT OVERHEAD".into(), "DOCK".into()],
            job: None,
        };
        let with = render_device(&device, &[], None, Some(&readout));
        let without = render_device(&device, &[], None, None);
        assert_ne!(with, without, "owning a kestrel changed nothing");

        // Cursor movement is visible feedback.
        let mut moved = Device::new();
        moved.open_list();
        moved.turn_page();
        moved.turn_page();
        moved.move_cursor(1, readout.rows.len());
        assert_ne!(
            render_device(&moved, &[], None, Some(&readout)),
            with,
            "the cursor does not draw"
        );
    }

    #[test]
    fn the_map_page_draws_the_country_and_says_so_without_one() {
        let mut device = Device::new();
        device.open_list();
        device.turn_page();

        let world = World::new(2024);
        let explored = MapState::new();
        let markers = [Marker {
            x: 0,
            z: 0,
            colour: map::colour::PLAYER,
            radius: 2,
        }];
        let country = Country {
            world: &world,
            explored: &explored,
            centre: (0, 0),
            markers: &markers,
        };

        let drawn = render_device(&device, &[], Some(&country), None);
        let blind = render_device(&device, &[], None, None);
        assert_ne!(drawn, blind, "the country did not draw");
        assert_eq!(drawn.len(), (DEVICE_WIDTH * DEVICE_HEIGHT * 4) as usize);
        // Deterministic, like every other panel.
        assert_eq!(drawn, render_device(&device, &[], Some(&country), None));

        // And the fleet page is a different picture again.
        let mut fleet = Device::new();
        fleet.open_list();
        assert_ne!(drawn, render_device(&fleet, &[], Some(&country), None));
    }

    // --------------------------------------------------------- holding it

    /// A camera looking down -Z from the origin, which is this engine's
    /// zero-yaw convention, with the basis the game hands the handheld.
    fn looking_north() -> (vx_render::Camera, Vec3, Vec3) {
        let camera = vx_render::Camera {
            position: glam::DVec3::new(0.0, 80.0, 0.0),
            yaw: 0.0,
            pitch: 0.0,
            aspect: 16.0 / 9.0,
            ..Default::default()
        };
        (camera, camera.forward(), camera.right())
    }

    #[test]
    fn the_raise_runs_both_ways_and_settles_at_the_ends() {
        let mut device = Device::new();
        assert_eq!(device.raise, 0.0);
        assert!(!device.showing());

        device.open_list();
        // A whole second is longer than the raise: it must not overshoot.
        for _ in 0..60 {
            device.raise_by(1.0 / 60.0);
        }
        assert_eq!(device.raise, 1.0);
        assert!(device.showing());

        device.close();
        // And it goes down rather than vanishing — the frame after closing,
        // there is still something on screen to draw.
        assert!(device.raise_by(1.0 / 60.0), "the unit blinked out of existence");
        for _ in 0..60 {
            device.raise_by(1.0 / 60.0);
        }
        assert_eq!(device.raise, 0.0);
        assert!(!device.showing());
    }

    #[test]
    fn the_carry_moves_the_unit_up_and_levels_it() {
        let (down_ahead, _, down_height, down_tilt) = carry(0.0);
        let (up_ahead, _, up_height, up_tilt) = carry(1.0);
        assert!(
            up_height > down_height,
            "raising it did not lift it: {down_height} to {up_height}"
        );
        assert!(up_tilt < down_tilt, "raising it did not level the glass");
        assert!(down_ahead > 0.0 && up_ahead > 0.0, "the unit is behind you");

        // Monotone in between, so nothing bounces on the way up.
        let mut last = carry(0.0).2;
        for step in 1..=20 {
            let height = carry(step as f32 / 20.0).2;
            assert!(height >= last - 1.0e-6, "the raise doubled back");
            last = height;
        }
    }

    #[test]
    fn the_screen_lands_in_front_of_the_camera_and_faces_it() {
        let (camera, forward, right) = looking_north();
        let corners = screen_corners(camera.local_position(), forward, right, 1.0);
        for corner in corners {
            let along = (corner - camera.local_position()).dot(forward);
            assert!(along > 0.05, "a corner of the glass is behind the eye");
            assert!(along < 1.5, "the glass is a metre and a half away");
        }
        // Top corners above the bottom ones, which is what stops the readout
        // arriving upside down.
        assert!(corners[0].y > corners[3].y);
        assert!(corners[1].y > corners[2].y);
    }

    #[test]
    fn the_projected_rect_is_on_screen_and_keeps_the_readouts_shape() {
        let (camera, forward, right) = looking_north();
        let size = (1280.0, 720.0);
        let corners = screen_corners(camera.local_position(), forward, right, 1.0);
        let rect = screen_rect(camera.view_projection(), corners, size)
            .expect("the raised screen projected to nothing");

        assert!(rect.x > 0.0 && rect.y > 0.0, "the screen ran off the frame");
        assert!(rect.x + rect.width < size.0);
        assert!(rect.y + rect.height < size.1);

        // Big enough to read: the overlay samples with `Nearest`, so a
        // rectangle much under the readout's own pixel size aliases the text
        // into porridge.
        assert!(
            rect.width >= DEVICE_WIDTH as f32,
            "the screen landed {} pixels wide, under the readout's own {}",
            rect.width,
            DEVICE_WIDTH
        );

        // And the shape is exactly the readout's: the rectangle is fitted
        // inside the projected face rather than stretched to it, so no
        // amount of tilt can squash the text.
        let want = DEVICE_WIDTH as f32 / DEVICE_HEIGHT as f32;
        let got = rect.width / rect.height;
        assert!(
            (got - want).abs() < 1.0e-3,
            "the screen is {got:.3} to one against the readout's {want:.3}"
        );
    }

    #[test]
    fn the_readout_keeps_its_shape_however_the_unit_is_tipped() {
        // The letterbox, checked across the whole raise: at every point in
        // the swing the rectangle is the readout's own shape, because a
        // screen that stretches as it comes up is worse than one that does
        // not move at all.
        let (camera, forward, right) = looking_north();
        let want = DEVICE_WIDTH as f32 / DEVICE_HEIGHT as f32;
        let mut seen = 0;
        for step in 0..=10 {
            let raise = step as f32 / 10.0;
            let corners = screen_corners(camera.local_position(), forward, right, raise);
            let Some(rect) =
                screen_rect(camera.view_projection(), corners, (1280.0, 720.0))
            else {
                continue;
            };
            seen += 1;
            assert!((rect.width / rect.height - want).abs() < 1.0e-3);
        }
        assert!(seen > 5, "only {seen} of the swing projected at all");
    }

    #[test]
    fn a_screen_behind_the_eye_projects_to_nothing() {
        // Turned around, the glass is behind the camera, and a rectangle
        // derived from a point behind the eye is a rectangle in the wrong
        // place. Nothing is the right answer.
        let (mut camera, _, _) = looking_north();
        let corners = screen_corners(camera.local_position(), camera.forward(), camera.right(), 1.0);
        camera.yaw = std::f32::consts::PI;
        assert!(screen_rect(camera.view_projection(), corners, (1280.0, 720.0)).is_none());
    }

    #[test]
    fn the_third_person_fallback_is_centred_and_the_right_shape() {
        let rect = centred(1280.0, 720.0);
        assert!((rect.x + rect.width / 2.0 - 640.0).abs() < 0.5);
        assert!((rect.y + rect.height / 2.0 - 360.0).abs() < 0.5);
        let want = DEVICE_WIDTH as f32 / DEVICE_HEIGHT as f32;
        assert!((rect.width / rect.height - want).abs() < 1.0e-3);
    }

    #[test]
    fn the_screen_comes_on_as_it_arrives() {
        let bright = [255u8, 255, 255, 200];
        let mut dark = bright;
        dim(&mut dark, 0.0);
        assert_eq!(dark[3], 0, "a stowed unit is showing a lit screen");
        assert_eq!(&dark[..3], &bright[..3], "dimming changed the colours");

        let mut half = bright;
        dim(&mut half, 0.5);
        assert!(half[3] > 0 && half[3] < bright[3]);

        let mut full = bright;
        dim(&mut full, 1.0);
        assert_eq!(full[3], bright[3], "a raised unit is not at full brightness");
    }
}
