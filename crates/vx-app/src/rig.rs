//! Composite rigs: machines built from a handful of cuboids.
//!
//! A rig is data — parts with a centre, a size, a tile and maybe a spin axis
//! — turned into [`Object`]s each frame. Every part rides the existing
//! instanced path, so per-object frustum culling and the shared depth buffer
//! keep working with zero new render code, and the inverse-transpose normal
//! fix is what makes these rotated, stretched parts light correctly.
//!
//! The digger deliberately reads like the classic dig-and-sell flash games:
//! squat rust-orange hull, chunky dark wheels, pale cab glass, and a tapered
//! steel drill on the nose that spins while she is cutting. The *vibe* is
//! borrowed; every pixel is ours.
//!
//! Rigs face local +X; [`yaw_towards`] turns a movement delta into the yaw
//! that points the nose along it.

use glam::{DVec3, Mat4, Vec3};
use vx_render::tiles::slot;
use vx_render::Object;

/// Which local axis a part spins around.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Spin {
    /// Along the nose — drill bits.
    Roll,
    /// Around vertical — rotors.
    Yaw,
}

/// One cuboid of a rig, in rig-local coordinates: +X forward, +Y up, origin
/// at ground level under the rig's centre.
#[derive(Debug, Clone, Copy)]
pub struct Part {
    pub centre: Vec3,
    pub size: Vec3,
    pub tile: u32,
    pub spin: Option<Spin>,
    /// Does this part slide with the gaze?
    ///
    /// Pupils, and nothing else. A whole extra transform stack for two
    /// cubes would be silly; a flag on the data is what a rig *is*.
    pub follow: bool,
}

impl Part {
    const fn fixed(centre: Vec3, size: Vec3, tile: u32) -> Self {
        Part {
            centre,
            size,
            tile,
            spin: None,
            follow: false,
        }
    }

    const fn spinning(centre: Vec3, size: Vec3, tile: u32, spin: Spin) -> Self {
        Part {
            centre,
            size,
            tile,
            spin: Some(spin),
            follow: false,
        }
    }

    /// A part that tracks whatever the rig is looking at.
    const fn following(centre: Vec3, size: Vec3, tile: u32) -> Self {
        Part {
            centre,
            size,
            tile,
            spin: None,
            follow: true,
        }
    }
}

/// How far a pupil may slide across the eye, in blocks. Small: the eye is a
/// quarter of a block wide, and a pupil that leaves it is a googly toy.
pub const EYE_TRAVEL: f32 = 0.035;

/// How far off its own nose a rig will *look* before giving up and turning.
///
/// The clamp is the whole of "roughly": eyes do not swivel into the back of
/// a head, so past this the gaze saturates and the body's own facing — which
/// `villagers::react` already turns toward whatever has their attention —
/// does the rest of the work.
pub const GAZE_YAW: f32 = 0.6;
pub const GAZE_PITCH: f32 = 0.4;

/// Where a rig is looking, as an offset from straight ahead in radians.
///
/// Clamped on construction, so nothing downstream has to remember to.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Gaze {
    pub yaw: f32,
    pub pitch: f32,
}

impl Gaze {
    /// Dead ahead: what everything that has no opinion looks at.
    pub const AHEAD: Gaze = Gaze {
        yaw: 0.0,
        pitch: 0.0,
    };

    /// The gaze of a body at `from`, facing `facing`, looking at `at`.
    pub fn towards(from: DVec3, facing: f32, at: DVec3) -> Gaze {
        // The difference is taken wide and narrowed: a gaze is a direction.
        let to = (at - from).as_vec3();
        let flat = (to.x * to.x + to.z * to.z).sqrt();
        if flat < 1.0e-3 {
            return Gaze::AHEAD;
        }
        // The rig's nose is +X at yaw zero, which is the convention
        // `yaw_towards` already sets; the difference of the two angles is
        // how far the eyes have to go.
        let bearing = (-to.z).atan2(to.x);
        let mut yaw = bearing - facing;
        // Into -PI..PI, so a target behind the left shoulder does not read
        // as one nearly a full turn to the right.
        while yaw > std::f32::consts::PI {
            yaw -= std::f32::consts::TAU;
        }
        while yaw < -std::f32::consts::PI {
            yaw += std::f32::consts::TAU;
        }
        Gaze {
            yaw: yaw.clamp(-GAZE_YAW, GAZE_YAW),
            pitch: (to.y / flat).atan().clamp(-GAZE_PITCH, GAZE_PITCH),
        }
    }
}

/// The handheld's screen face, in rig-local coordinates.
///
/// Exported because two things have to agree about it and neither should be
/// guessing: [`Rig::handheld`] builds the plate here, and
/// [`crate::device::screen_corners`] projects the readout onto the same four
/// corners. A model whose screen is somewhere other than where the readout
/// lands is the one bug this design can have, so there is one set of numbers.
pub mod screen {
    /// How far forward of the case's centre the glass sits — negative,
    /// because the rig's nose is +X and the screen faces back at the person
    /// holding it.
    pub const DEPTH: f32 = -0.056;
    /// Half-width across the face, along the rig's lateral axis.
    pub const HALF_WIDTH: f32 = 0.205;
    /// Half-height. Sized against the readout it carries — 240 by 166 — so
    /// the text lands square rather than stretched.
    pub const HALF_HEIGHT: f32 = HALF_WIDTH * 166.0 / 240.0;
}

/// A machine's shape.
#[derive(Debug, Clone)]
pub struct Rig {
    pub parts: Vec<Part>,
}

impl Rig {
    /// The ground digger, Motherload-flavoured and original.
    pub fn digger() -> Self {
        let hull = slot::HULL;
        let tread = slot::TREAD;
        let steel = slot::STEEL;
        let cab = slot::CAB;
        Rig {
            parts: vec![
                // Hull and cab.
                Part::fixed(Vec3::new(-0.05, 0.45, 0.0), Vec3::new(0.95, 0.5, 0.68), hull),
                Part::fixed(Vec3::new(-0.14, 0.79, 0.0), Vec3::new(0.44, 0.26, 0.5), cab),
                // The nose drill, tapering, both stages spin.
                Part::spinning(Vec3::new(0.58, 0.45, 0.0), Vec3::new(0.34, 0.3, 0.3), steel, Spin::Roll),
                Part::spinning(Vec3::new(0.85, 0.45, 0.0), Vec3::new(0.22, 0.16, 0.16), steel, Spin::Roll),
                // Four chunky wheels.
                Part::fixed(Vec3::new(0.28, 0.16, 0.38), Vec3::new(0.32, 0.32, 0.14), tread),
                Part::fixed(Vec3::new(0.28, 0.16, -0.38), Vec3::new(0.32, 0.32, 0.14), tread),
                Part::fixed(Vec3::new(-0.34, 0.16, 0.38), Vec3::new(0.32, 0.32, 0.14), tread),
                Part::fixed(Vec3::new(-0.34, 0.16, -0.38), Vec3::new(0.32, 0.32, 0.14), tread),
            ],
        }
    }

    /// The flying drone: hull, skids, and a rotor that spins overhead.
    pub fn flier() -> Self {
        Rig {
            parts: vec![
                Part::fixed(Vec3::new(0.0, 0.38, 0.0), Vec3::new(0.9, 0.4, 0.6), slot::HULL),
                Part::fixed(Vec3::new(0.22, 0.64, 0.0), Vec3::new(0.34, 0.2, 0.42), slot::CAB),
                Part::spinning(Vec3::new(0.0, 0.86, 0.0), Vec3::new(1.5, 0.06, 0.16), slot::STEEL, Spin::Yaw),
                Part::fixed(Vec3::new(0.0, 0.05, 0.3), Vec3::new(0.85, 0.08, 0.1), slot::TREAD),
                Part::fixed(Vec3::new(0.0, 0.05, -0.3), Vec3::new(0.85, 0.08, 0.1), slot::TREAD),
            ],
        }
    }

    /// What is left of a machine.
    ///
    /// The machine's own rig with two changes, and both are the point: every
    /// part is repainted to [`slot::WRECK`], and **the spinning parts are
    /// gone**. A rotor or a nose drill is the first thing to shear off and
    /// the first thing an eye looks for, so a hulk with no rotor reads as
    /// broken from across a valley without a label on it.
    ///
    /// Built from the live rig rather than authored separately, so a machine
    /// that gains a part gains it in its own wreck too. A second hand-written
    /// model would drift the first time anybody touched the first one.
    pub fn wreck(of: Rig) -> Self {
        Rig {
            parts: of
                .parts
                .into_iter()
                .filter(|part| part.spin.is_none())
                .map(|part| Part {
                    tile: slot::WRECK,
                    ..part
                })
                .collect(),
        }
    }

    /// How far a wreck lies over. Not upright, not flat: a machine that came
    /// down hard rests on a corner.
    pub const WRECK_TILT: f32 = 0.55;

    /// A villager: boots, trousers, jacket, arms, a head and a face, with the
    /// proportions nudged per variant so the town is not staffed by clones.
    ///
    /// The deputies and the shelters' holders wear this rig too, so they get
    /// the face for nothing — with the gaze centred, because nobody has asked
    /// them where they are looking yet.
    pub fn villager(variant: usize) -> Self {
        // Deterministic small nudges, one per variant.
        let stretch = match variant % 3 {
            1 => 1.08,
            2 => 0.92,
            _ => 1.0,
        };
        let girth = match variant % 3 {
            2 => 1.18,
            _ => 1.0,
        };
        let jacket = match variant % 3 {
            1 => slot::HULL, // one of them wears work orange
            _ => slot::CLOTH,
        };
        let leg = 0.62 * stretch;
        let torso_h = 0.52 * stretch;
        let torso_top = leg + torso_h;
        /// The plane the face sits on: the front of the head, plus enough to
        /// clear it.
        const FACE: f32 = 0.125;
        Rig {
            parts: vec![
                // Legs.
                Part::fixed(
                    Vec3::new(0.0, leg * 0.5, 0.09),
                    Vec3::new(0.15, leg, 0.13) * Vec3::new(girth, 1.0, 1.0),
                    slot::TREAD,
                ),
                Part::fixed(
                    Vec3::new(0.0, leg * 0.5, -0.09),
                    Vec3::new(0.15, leg, 0.13) * Vec3::new(girth, 1.0, 1.0),
                    slot::TREAD,
                ),
                // Torso.
                Part::fixed(
                    Vec3::new(0.0, leg + torso_h * 0.5, 0.0),
                    Vec3::new(0.24, torso_h, 0.4) * Vec3::new(girth, 1.0, girth),
                    jacket,
                ),
                // Arms, hanging at the sides.
                Part::fixed(
                    Vec3::new(0.0, leg + torso_h * 0.55, 0.26 * girth),
                    Vec3::new(0.11, torso_h * 0.9, 0.1),
                    jacket,
                ),
                Part::fixed(
                    Vec3::new(0.0, leg + torso_h * 0.55, -0.26 * girth),
                    Vec3::new(0.11, torso_h * 0.9, 0.1),
                    jacket,
                ),
                // Head.
                Part::fixed(
                    Vec3::new(0.0, torso_top + 0.15, 0.0),
                    Vec3::new(0.24, 0.28, 0.24),
                    slot::SKIN,
                ),
                // The face, on the front of the head — five small cubes that
                // do more for a town than any other five cubes in this game.
                // A bare skin block for a head reads as a mannequin; two eyes
                // and a mouth read as somebody who might have an opinion
                // about you, which is what the whole townsfolk round was for.
                //
                // Whites first, standing a hair proud of the skin so they
                // never z-fight with it.
                Part::fixed(
                    Vec3::new(FACE, torso_top + 0.19, 0.055),
                    Vec3::new(0.03, 0.07, 0.07),
                    slot::EYE,
                ),
                Part::fixed(
                    Vec3::new(FACE, torso_top + 0.19, -0.055),
                    Vec3::new(0.03, 0.07, 0.07),
                    slot::EYE,
                ),
                // Pupils, which are the parts that move.
                Part::following(
                    Vec3::new(FACE + 0.012, torso_top + 0.19, 0.055),
                    Vec3::new(0.02, 0.035, 0.035),
                    slot::PUPIL,
                ),
                Part::following(
                    Vec3::new(FACE + 0.012, torso_top + 0.19, -0.055),
                    Vec3::new(0.02, 0.035, 0.035),
                    slot::PUPIL,
                ),
                // And a mouth: one dark line, no expression. Anything more
                // would be animation this game has no rig for, and a flat
                // line at least never smiles at the wrong moment.
                Part::fixed(
                    Vec3::new(FACE, torso_top + 0.08, 0.0),
                    Vec3::new(0.02, 0.015, 0.09),
                    slot::PUPIL,
                ),
            ],
        }
    }

    /// The thing that comes when a mine runs loud.
    ///
    /// Built wrong on purpose: long low body, too many legs for its size,
    /// no head worth the name. It reads as *not a person* at fifty blocks in
    /// lamplight, which is the only job the shape has — every other thing
    /// with legs in this game is somebody you could talk to.
    pub fn stalker() -> Self {
        let hip = 0.9;
        let body = 1.4;
        Rig {
            parts: vec![
                // Four long legs, splayed.
                Part::fixed(
                    Vec3::new(0.34, hip * 0.5, 0.30),
                    Vec3::new(0.10, hip, 0.10),
                    slot::TREAD,
                ),
                Part::fixed(
                    Vec3::new(0.34, hip * 0.5, -0.30),
                    Vec3::new(0.10, hip, 0.10),
                    slot::TREAD,
                ),
                Part::fixed(
                    Vec3::new(-0.34, hip * 0.55, 0.30),
                    Vec3::new(0.10, hip * 1.1, 0.10),
                    slot::TREAD,
                ),
                Part::fixed(
                    Vec3::new(-0.34, hip * 0.55, -0.30),
                    Vec3::new(0.10, hip * 1.1, 0.10),
                    slot::TREAD,
                ),
                // The body: long, low, and slung between them.
                Part::fixed(
                    Vec3::new(0.0, hip + 0.18, 0.0),
                    Vec3::new(body, 0.34, 0.5),
                    slot::BUNKER_SHELL,
                ),
                // A blunt snout where a head should be.
                Part::fixed(
                    Vec3::new(body * 0.55, hip + 0.16, 0.0),
                    Vec3::new(0.34, 0.24, 0.3),
                    slot::RUSTED_METAL,
                ),
            ],
        }
    }

    /// The player, seen from behind in third person.
    ///
    /// Deliberately not a villager: same build, but a hi-vis work jacket, a
    /// hard hat, and the boring drill slung at the hip, so at a glance you
    /// can tell yourself from the townsfolk in a crowd.
    pub fn player() -> Self {
        let leg = 0.64;
        let torso_h = 0.56;
        let torso_top = leg + torso_h;
        Rig {
            parts: vec![
                // Legs.
                Part::fixed(Vec3::new(0.0, leg * 0.5, 0.10), Vec3::new(0.16, leg, 0.14), slot::TREAD),
                Part::fixed(Vec3::new(0.0, leg * 0.5, -0.10), Vec3::new(0.16, leg, 0.14), slot::TREAD),
                // Hi-vis torso.
                Part::fixed(
                    Vec3::new(0.0, leg + torso_h * 0.5, 0.0),
                    Vec3::new(0.26, torso_h, 0.42),
                    slot::HULL,
                ),
                // Arms.
                Part::fixed(
                    Vec3::new(0.0, leg + torso_h * 0.55, 0.27),
                    Vec3::new(0.12, torso_h * 0.9, 0.11),
                    slot::HULL,
                ),
                Part::fixed(
                    Vec3::new(0.0, leg + torso_h * 0.55, -0.27),
                    Vec3::new(0.12, torso_h * 0.9, 0.11),
                    slot::HULL,
                ),
                // Head, then the hard hat on top of it.
                Part::fixed(Vec3::new(0.0, torso_top + 0.14, 0.0), Vec3::new(0.24, 0.26, 0.24), slot::SKIN),
                Part::fixed(Vec3::new(0.0, torso_top + 0.30, 0.0), Vec3::new(0.30, 0.10, 0.30), slot::HULL),
                // The drill on the hip, so the silhouette says "miner".
                Part::fixed(
                    Vec3::new(0.10, leg + 0.06, 0.26),
                    Vec3::new(0.30, 0.13, 0.12),
                    slot::STEEL,
                ),
            ],
        }
    }

    /// The same rig squashed vertically.
    ///
    /// Stance poses without an animation system: a crouching body is a standing
    /// body with its height scaled, which is crude but honest — the parts are
    /// boxes and there are no joints to bend. It reads correctly in third
    /// person, which is the only place anyone sees it.
    pub fn compressed(&self, factor: f32) -> Self {
        let factor = factor.clamp(0.05, 1.0);
        Rig {
            parts: self
                .parts
                .iter()
                .map(|part| {
                    let mut part = *part;
                    part.centre.y *= factor;
                    part.size.y *= factor;
                    part
                })
                .collect(),
        }
    }

    /// The player's handheld compact boring drill, sized for the viewmodel.
    pub fn hand_drill() -> Self {
        Rig {
            parts: vec![
                Part::fixed(Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.34, 0.2, 0.16), slot::HULL),
                Part::fixed(Vec3::new(-0.08, -0.17, 0.0), Vec3::new(0.1, 0.18, 0.1), slot::TREAD),
                Part::spinning(Vec3::new(0.26, 0.0, 0.0), Vec3::new(0.2, 0.1, 0.1), slot::STEEL, Spin::Roll),
                Part::spinning(Vec3::new(0.41, 0.0, 0.0), Vec3::new(0.12, 0.05, 0.05), slot::STEEL, Spin::Roll),
            ],
        }
    }

    /// The kestrel: the flier's silhouette at a quarter of the size — the
    /// same machine to the eye, which is the point of the symmetric tech.
    /// A hull sliver, a spinning rotor disc, two stub skids.
    pub fn kestrel() -> Self {
        Rig {
            parts: vec![
                Part::fixed(Vec3::new(0.0, 0.14, 0.0), Vec3::new(0.34, 0.12, 0.2), slot::HULL),
                Part::fixed(Vec3::new(0.16, 0.16, 0.0), Vec3::new(0.1, 0.08, 0.12), slot::CAB),
                Part::spinning(Vec3::new(0.0, 0.24, 0.0), Vec3::new(0.44, 0.02, 0.06), slot::STEEL, Spin::Yaw),
                Part::fixed(Vec3::new(0.0, 0.04, -0.08), Vec3::new(0.2, 0.04, 0.03), slot::TREAD),
                Part::fixed(Vec3::new(0.0, 0.04, 0.08), Vec3::new(0.2, 0.04, 0.03), slot::TREAD),
            ],
        }
    }

    /// The handheld PC, held in both hands and raised into view.
    ///
    /// Built around the screen rather than around a silhouette: the glass is
    /// the reason the object exists, so [`screen`] fixes where it sits and
    /// everything else — case, bezel, dials, the strap over the forearm — is
    /// arranged around it. The plate itself is dark on purpose: the readout
    /// is drawn over it by the overlay pass, so what this rig contributes is
    /// the frame that makes a rectangle of text read as a thing somebody is
    /// holding.
    ///
    /// Local axes as ever: +X is the nose, which here points away from the
    /// face, so the screen sits at a negative X and looks back at you. Like
    /// the drill and the launcher it hangs off the camera, so parts sit at
    /// negative Y and the ground-origin convention does not apply.
    /// A trunk on its way down.
    ///
    /// Built along +X from the hinge, so `objects_pitched` swings it about
    /// its own base — the same transform the handheld rides on, and no
    /// renderer change at all. The pitch to pass is `π/2 − angle`, because a
    /// stem at angle zero is standing up and the rig lies along +X.
    pub fn trunk(height: i32, thickness: f32, tile: u32, crown: u32) -> Self {
        let length = height as f32;
        let segments = height.clamp(2, 24);
        let step = length / segments as f32;
        let mut parts = Vec::with_capacity(segments as usize + 1);
        for index in 0..segments {
            // Tapered: a stem is thicker at the butt than at the tip.
            let along = (index as f32 + 0.5) * step;
            let taper = 1.0 - 0.45 * (along / length);
            parts.push(Part::fixed(
                Vec3::new(along, 0.0, 0.0),
                Vec3::new(step, thickness * taper, thickness * taper),
                tile,
            ));
        }
        // The crown, still on it. A tree does not shed its head on the way
        // down.
        parts.push(Part::fixed(
            Vec3::new(length * 0.9, 0.0, 0.0),
            Vec3::splat(thickness * 3.5),
            crown,
        ));
        Rig { parts }
    }

    pub fn handheld() -> Self {
        let width = screen::HALF_WIDTH;
        let height = screen::HALF_HEIGHT;
        Rig {
            parts: vec![
                // The case: a slab a little larger than the glass, thick
                // enough to read as a machine from the side.
                Part::fixed(
                    Vec3::new(0.0, 0.0, 0.0),
                    Vec3::new(0.11, height * 2.0 + 0.09, width * 2.0 + 0.09),
                    slot::HULL,
                ),
                // The bezel: brushed metal between case and glass.
                Part::fixed(
                    Vec3::new(screen::DEPTH * 0.55, 0.0, 0.0),
                    Vec3::new(0.02, height * 2.0 + 0.05, width * 2.0 + 0.05),
                    slot::STEEL,
                ),
                // The glass. Exactly the screen rectangle, a hair proud of
                // the bezel so it never fights it for depth.
                Part::fixed(
                    Vec3::new(screen::DEPTH, 0.0, 0.0),
                    Vec3::new(0.012, height * 2.0, width * 2.0),
                    slot::CAB,
                ),
                // Two dials under the glass, because a machine with no
                // controls on it is a picture of a machine.
                Part::fixed(
                    Vec3::new(screen::DEPTH * 0.8, -height - 0.035, width * 0.55),
                    Vec3::new(0.03, 0.035, 0.035),
                    slot::STEEL,
                ),
                Part::fixed(
                    Vec3::new(screen::DEPTH * 0.8, -height - 0.035, width * 0.2),
                    Vec3::new(0.03, 0.035, 0.035),
                    slot::STEEL,
                ),
                // A stub aerial off the top corner.
                Part::fixed(
                    Vec3::new(0.0, height + 0.09, -width * 0.8),
                    Vec3::new(0.02, 0.11, 0.02),
                    slot::STEEL,
                ),
                // The strap over the forearm: two bands and a backing plate.
                Part::fixed(
                    Vec3::new(0.055, -0.02, width * 0.62),
                    Vec3::new(0.05, height * 1.5, 0.04),
                    slot::CLOTH,
                ),
                Part::fixed(
                    Vec3::new(0.055, -0.02, -width * 0.62),
                    Vec3::new(0.05, height * 1.5, 0.04),
                    slot::CLOTH,
                ),
                Part::fixed(
                    Vec3::new(0.075, -0.02, 0.0),
                    Vec3::new(0.03, height * 1.2, width * 1.1),
                    slot::TREAD,
                ),
            ],
        }
    }

    /// The slug launcher, sized for the viewmodel: a fat barrel over a boxy
    /// receiver, all mass and no grace. Like the drill it hangs off the
    /// camera, so parts sit at negative Y and the ground-origin convention
    /// does not apply.
    pub fn launcher() -> Self {
        Rig {
            parts: vec![
                // The receiver, shoulder-height and square.
                Part::fixed(Vec3::new(-0.02, -0.02, 0.0), Vec3::new(0.36, 0.16, 0.14), slot::HULL),
                // The barrel: wide bore, short throw.
                Part::fixed(Vec3::new(0.28, 0.01, 0.0), Vec3::new(0.3, 0.11, 0.11), slot::STEEL),
                // The muzzle ring, a shade wider than the barrel.
                Part::fixed(Vec3::new(0.44, 0.01, 0.0), Vec3::new(0.05, 0.14, 0.14), slot::STEEL),
                // The grip.
                Part::fixed(Vec3::new(-0.1, -0.18, 0.0), Vec3::new(0.09, 0.16, 0.09), slot::TREAD),
                // The satchel feed under the receiver.
                Part::fixed(Vec3::new(0.06, -0.14, 0.0), Vec3::new(0.14, 0.1, 0.12), slot::TREAD),
            ],
        }
    }

    /// Build this frame's objects: the rig at `position` (ground point under
    /// its centre), nose yawed by `yaw`, spinning parts rolled by `spin`.
    pub fn objects(&self, position: Vec3, yaw: f32, spin: f32) -> Vec<Object> {
        self.objects_pitched(position, yaw, 0.0, spin)
    }

    /// [`Rig::objects`], with the eyes pointed somewhere.
    ///
    /// Only parts marked [`Part::follow`] move, so every rig without a face
    /// draws exactly as it always did — which is why the plain `objects`
    /// above is still the call almost everything makes.
    pub fn objects_looking(&self, position: Vec3, yaw: f32, spin: f32, gaze: Gaze) -> Vec<Object> {
        self.build(position, yaw, 0.0, spin, gaze)
    }

    /// [`Rig::objects`] with the nose also pitched up (positive) or down.
    /// Ground rigs stay level; the handheld drill follows the player's gaze.
    pub fn objects_pitched(&self, position: Vec3, yaw: f32, pitch: f32, spin: f32) -> Vec<Object> {
        self.build(position, yaw, pitch, spin, Gaze::AHEAD)
    }

    fn build(&self, position: Vec3, yaw: f32, pitch: f32, spin: f32, gaze: Gaze) -> Vec<Object> {
        let place = Mat4::from_translation(position)
            * Mat4::from_rotation_y(yaw)
            * Mat4::from_rotation_z(pitch);
        self.parts
            .iter()
            .map(|part| {
                let turn = match part.spin {
                    Some(Spin::Roll) => Mat4::from_rotation_x(spin),
                    Some(Spin::Yaw) => Mat4::from_rotation_y(spin),
                    None => Mat4::IDENTITY,
                };
                // A following part slides across the face rather than
                // rotating: the eye is flat, and a rotated pupil at this
                // scale is one texel of difference and a lot of maths.
                let centre = if part.follow {
                    part.centre
                        + Vec3::new(
                            0.0,
                            gaze.pitch / GAZE_PITCH * EYE_TRAVEL,
                            -gaze.yaw / GAZE_YAW * EYE_TRAVEL,
                        )
                } else {
                    part.centre
                };
                // Unit cube spans 0..1: centre it, scale it, spin it, place it.
                let model = place
                    * Mat4::from_translation(centre)
                    * turn
                    * Mat4::from_scale(part.size)
                    * Mat4::from_translation(Vec3::splat(-0.5));
                Object::new(model, part.tile)
            })
            .collect()
    }
}

/// The yaw that points a rig's nose (+X) along a movement delta, or `None`
/// when the delta is too small to mean anything — callers keep the last yaw,
/// so a parked rig does not twitch.
pub fn yaw_towards(dx: f32, dz: f32) -> Option<f32> {
    if dx * dx + dz * dz < 1.0e-6 {
        return None;
    }
    Some((-dz).atan2(dx))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_gaze_saturates_rather_than_swivelling_into_the_back_of_a_head() {
        // The clamp is the whole of "roughly". A target behind somebody has
        // to read as "as far round as the eyes go", not as a pupil in an
        // ear — and it must never wrap the wrong way round the circle.
        let at = Vec3::new(0.0, 1.6, 0.0);
        let ahead = Gaze::towards(at.as_dvec3(), 0.0, (at + Vec3::new(4.0, 0.0, 0.0)).as_dvec3());
        assert!(ahead.yaw.abs() < 1.0e-3, "looking down its own nose was {ahead:?}");

        let behind = Gaze::towards(at.as_dvec3(), 0.0, (at + Vec3::new(-4.0, 0.0, 0.0)).as_dvec3());
        assert_eq!(behind.yaw.abs(), GAZE_YAW, "the gaze did not saturate");

        for turn in [-3.0f32, -1.2, -0.3, 0.0, 0.7, 2.5, 3.1] {
            for side in [-1.0f32, 1.0] {
                let target = at + Vec3::new(turn.cos() * 5.0, 0.0, side * 5.0);
                let gaze = Gaze::towards(at.as_dvec3(), 0.0, (target).as_dvec3());
                assert!(gaze.yaw.abs() <= GAZE_YAW + 1.0e-6);
                assert!(gaze.pitch.abs() <= GAZE_PITCH + 1.0e-6);
            }
        }

        // And up is up: somebody on a roof pulls the eyes up, not down.
        let above = Gaze::towards(at.as_dvec3(), 0.0, (at + Vec3::new(2.0, 4.0, 0.0)).as_dvec3());
        assert!(above.pitch > 0.0, "the eyes looked away from a target overhead");
    }

    #[test]
    fn only_the_pupils_move_with_the_gaze() {
        // The promise that lets every other rig in the game keep calling
        // `objects` unchanged: a gaze moves two cubes and nothing else.
        let rig = Rig::villager(0);
        let still = rig.objects_looking(Vec3::ZERO, 0.0, 0.0, Gaze::AHEAD);
        let looking = rig.objects_looking(
            Vec3::ZERO,
            0.0,
            0.0,
            Gaze {
                yaw: GAZE_YAW,
                pitch: GAZE_PITCH,
            },
        );
        let moved = still
            .iter()
            .zip(&looking)
            .filter(|(before, after)| before.model != after.model)
            .count();
        assert_eq!(moved, 2, "{moved} parts moved when the eyes did");

        // A rig with no face is entirely unmoved by being asked to look.
        let digger = Rig::digger();
        assert_eq!(
            digger.objects(Vec3::ZERO, 0.0, 0.0),
            digger.objects_looking(Vec3::ZERO, 0.0, 0.0, Gaze { yaw: 0.5, pitch: 0.3 }),
        );
    }

    #[test]
    fn rig_shapes_are_pinned() {
        // Part counts and spin counts are the cheap regression net for
        // "someone deleted a wheel".
        let digger = Rig::digger();
        assert_eq!(digger.parts.len(), 8);
        assert_eq!(digger.parts.iter().filter(|part| part.spin.is_some()).count(), 2);

        let flier = Rig::flier();
        assert_eq!(flier.parts.len(), 5);
        assert_eq!(flier.parts.iter().filter(|part| part.spin == Some(Spin::Yaw)).count(), 1);

        assert_eq!(Rig::hand_drill().parts.len(), 4);
        // Case, bezel, glass, two dials, an aerial and three of strap.
        let handheld = Rig::handheld();
        assert_eq!(handheld.parts.len(), 9);
        assert!(handheld.parts.iter().all(|part| part.spin.is_none()));
        // Four legs and a body that is not a person's.
        let stalker = Rig::stalker();
        assert_eq!(stalker.parts.len(), 6);
        assert_eq!(stalker.parts.iter().filter(|part| part.spin.is_some()).count(), 0);
        assert_eq!(Rig::launcher().parts.len(), 5);
        let kestrel = Rig::kestrel();
        assert_eq!(kestrel.parts.len(), 5);
        assert_eq!(
            kestrel.parts.iter().filter(|part| part.spin == Some(Spin::Yaw)).count(),
            1
        );

        let player = Rig::player();
        assert_eq!(player.parts.len(), 8);
        assert!(player.parts.iter().all(|part| part.spin.is_none()), "the player does not spin");

        for variant in 0..3 {
            let villager = Rig::villager(variant);
            assert_eq!(villager.parts.len(), 11, "six of body and five of face");
            assert!(villager.parts.iter().all(|part| part.spin.is_none()));
            assert_eq!(villager.parts.iter().filter(|part| part.follow).count(), 2);
        }
        // The variants must actually look different.
        let a = Rig::villager(0).objects(Vec3::ZERO, 0.0, 0.0);
        let b = Rig::villager(1).objects(Vec3::ZERO, 0.0, 0.0);
        assert_ne!(a[2].model, b[2].model, "variant 1 is a clone of variant 0");
    }

    #[test]
    fn objects_track_position_yaw_and_spin() {
        let rig = Rig::digger();
        let here = rig.objects(Vec3::new(10.0, 60.0, 5.0), 0.0, 0.0);
        assert_eq!(here.len(), rig.parts.len());

        // Moving the rig moves every part.
        let there = rig.objects(Vec3::new(11.0, 60.0, 5.0), 0.0, 0.0);
        for (a, b) in here.iter().zip(&there) {
            assert!((b.bounds_min.x - a.bounds_min.x - 1.0).abs() < 1.0e-4);
        }

        // Yaw and spin each change the transforms.
        let yawed = rig.objects(Vec3::new(10.0, 60.0, 5.0), 1.0, 0.0);
        assert_ne!(here[2].model, yawed[2].model);
        let spun = rig.objects(Vec3::new(10.0, 60.0, 5.0), 0.0, 1.0);
        assert_ne!(here[2].model, spun[2].model, "the drill part should spin");
        assert_eq!(here[0].model, spun[0].model, "the hull must not spin");
    }

    #[test]
    fn every_part_sits_above_the_ground_point() {
        // position.y is the ground under the rig; nothing may dip below it,
        // or machines look buried in flat terrain.
        for rig in [
            Rig::digger(),
            Rig::flier(),
            Rig::player(),
            Rig::villager(0),
            Rig::villager(1),
            Rig::villager(2),
        ] {
            for object in rig.objects(Vec3::new(0.0, 50.0, 0.0), 0.7, 0.3) {
                assert!(
                    object.bounds_min.y >= 50.0 - 0.02,
                    "a part dips to {} below the ground point",
                    object.bounds_min.y
                );
            }
        }
    }

    #[test]
    fn pitch_tips_the_nose_toward_the_gaze() {
        let rig = Rig::hand_drill();
        let level = rig.objects_pitched(Vec3::ZERO, 0.0, 0.0, 0.0);
        let down = rig.objects_pitched(Vec3::ZERO, 0.0, -0.8, 0.0);
        // The bit is the last part and sits furthest along +X; pitching down
        // must carry it below its level height.
        let bit = rig.parts.len() - 1;
        assert!(
            down[bit].bounds_max.y < level[bit].bounds_min.y,
            "a downward pitch left the bit at {} (level bottom {})",
            down[bit].bounds_max.y,
            level[bit].bounds_min.y
        );
    }

    #[test]
    fn yaw_points_the_nose_along_travel() {
        // Rig forward is +X; check the four cardinals through the actual
        // rotation rather than trusting the trig by eye.
        for (dx, dz) in [(1.0f32, 0.0f32), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
            let yaw = yaw_towards(dx, dz).expect("a unit delta has a yaw");
            let nose = Mat4::from_rotation_y(yaw).transform_vector3(Vec3::X);
            assert!(
                (nose.x - dx).abs() < 1.0e-5 && (nose.z - dz).abs() < 1.0e-5,
                "delta ({dx},{dz}) yawed the nose to {nose:?}"
            );
        }
        assert!(yaw_towards(0.0, 0.0).is_none(), "a parked rig must keep its yaw");
    }
    /// **The same body draws the same bytes sixteen hundred kilometres
    /// out.** A rig built from `Camera::relative` at the origin and at a
    /// hundred thousand chunks out — camera and body moved by the same
    /// offset — produces bit-identical model matrices, which is the render
    /// test's byte-identical frame without a GPU: nothing in the draw path
    /// ever holds an absolute `f32`.
    #[test]
    fn the_same_body_draws_the_same_bytes_sixteen_hundred_kilometres_out() {
        let far_blocks = 100_000.0 * f64::from(vx_core::CHUNK_SIZE);
        let far = DVec3::new(far_blocks, 0.0, far_blocks);
        let scene = |offset: DVec3| {
            let camera = vx_render::Camera {
                position: DVec3::new(3.25, 71.5, -2.75) + offset,
                ..vx_render::Camera::default()
            };
            let body = DVec3::new(1.375, 65.0, -9.625) + offset;
            let eye = body + DVec3::new(0.0, 1.4, 0.0);
            let gaze = Gaze::towards(eye, 0.7, camera.position);
            Rig::player()
                .objects_looking(camera.relative(body), 0.7, 0.3, gaze)
                .into_iter()
                .map(Object::already_relative)
                .collect::<Vec<_>>()
        };
        let here = scene(DVec3::ZERO);
        let there = scene(far);
        assert_eq!(here.len(), there.len());
        assert!(here.len() > 4, "the player rig has parts");
        for (near, distant) in here.iter().zip(&there) {
            let a: Vec<u32> = near.model.to_cols_array().iter().map(|v| v.to_bits()).collect();
            let b: Vec<u32> = distant.model.to_cols_array().iter().map(|v| v.to_bits()).collect();
            assert_eq!(a, b, "a part's matrix differs far out");
            assert_eq!(near.bounds_min.to_array().map(f32::to_bits), distant.bounds_min.to_array().map(f32::to_bits));
            assert_eq!(near.bounds_max.to_array().map(f32::to_bits), distant.bounds_max.to_array().map(f32::to_bits));
            assert!(near.relative && distant.relative);
        }
    }

}
