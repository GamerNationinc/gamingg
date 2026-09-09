//! The handheld drill's laws: how fast the bit chews, what a hold looks like
//! on the way through, and where the block goes when it lets go.
//!
//! # Why this is not in `main.rs` any more
//!
//! `App::update_drilling` is the loop the whole game is about — you hold the
//! trigger, the rock gives up, the ore goes on the pile — and until stage 48
//! it was also the one verb in that loop with **no test**. Not because it is
//! hard to test, but because it is a method on `App`, which owns a `Window`;
//! `main.rs` has no test module and cannot have a useful one. So the drill's
//! arithmetic was unreachable, and the fact that a block mined with no base
//! container vanished without a word went unnoticed for twenty-odd stages.
//!
//! What moved here is only the *arithmetic*: the power, the bite, the four
//! layers a worked face shows, and the deposit rule. Everything that is
//! *policy* — whether the launcher is out, who hears the drill running, what a
//! lockbox does differently, when a trunk is a tree rather than a block, who
//! saw you, what a refusal costs — stays in `App` where it belongs, because
//! all of it reads state this module has no business knowing about.
//!
//! The point of the split is that [`crate::session`] drills with **the same
//! four functions the live game drills with**. A playthrough that re-implemented
//! the drill would be a test of the test.

use vx_agent::Stockpile;
use vx_core::{BlockId, BlockRegistry, Face};

/// Quarters of the way through, each of which takes a layer of cells off the
/// face being worked. Four is what makes a face being drilled *look* drilled
/// without changing when the block finishes.
pub const LAYERS: f32 = 4.0;

/// Hardness-units per second the bit chews, given what you know and what you
/// have bought.
///
/// The two multiply rather than add: a skill level is worth more to somebody
/// who paid for the drill, which is the shape every other pairing in the game
/// uses.
pub fn power_of(mining_level: u32, drill_upgrade: u32) -> f32 {
    crate::skills::drill_power(mining_level) * crate::wallet::drill_multiplier(drill_upgrade)
}

/// How much of a block a single frame of holding takes off it.
///
/// The floor on hardness is what stops a nearly-free block dividing by nearly
/// nothing and finishing in one frame with a visible pop.
pub fn bite(hardness: f32, power: f32, dt: f32) -> f32 {
    dt * power / hardness.max(0.05)
}

/// Seconds of unbroken holding a block of this hardness costs.
///
/// The inverse of [`bite`], and the number a player actually feels. Copper ore
/// is 2.5 hard and a fresh drill is 1.25 a second, so ore is two seconds a
/// block on the day you arrive — which is the pace the whole opening is
/// balanced around.
pub fn seconds_for(hardness: f32, power: f32) -> f32 {
    hardness.max(0.05) / power
}

/// Where one frame of holding left the block.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bite {
    /// How far through it is now.
    pub progress: f32,
    /// Whether this frame crossed a quarter and owes the face a layer.
    pub carve: bool,
    /// Whether the block is through and should be broken.
    pub through: bool,
}

/// Advance a hold by one frame.
///
/// `before` is `Some(progress)` when the bit is still on the block it was on
/// last frame, and `None` when the aim has moved — which is the whole rule
/// behind ordinary drilling: look away and you start again. (A lockbox is the
/// deliberate exception, and it keeps its own progress in `permits`, not here.)
pub fn advance_bite(before: Option<f32>, step: f32) -> Bite {
    // A fresh target is clamped on its first frame and a continuing one is
    // not. That asymmetry is load-bearing: it is what stops a single enormous
    // `dt` — an alt-tab, a stalled frame — from carrying a brand new block
    // straight past 1.0 without ever drawing a worked face.
    let progress = match before {
        Some(carried) => carried + step,
        None => step.min(1.0),
    };
    let layers = |value: f32| (value * LAYERS).floor() as i32;
    Bite {
        progress,
        carve: layers(progress) > layers(before.unwrap_or(0.0)) && progress < 1.0,
        through: progress >= 1.0,
    }
}

/// The index `vx_world::micro::Shape` wants for a face.
pub fn face_index(face: Face) -> usize {
    Face::ALL
        .iter()
        .position(|other| *other == face)
        .unwrap_or(0)
}

/// What became of a block that came out of the ground.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Deposited {
    /// It went on the pile, by this name.
    Piled(String),
    /// A supply cache pays out its own haul rather than yielding one
    /// crate-shaped block, so the crate itself is deliberately nothing.
    Crate,
    /// There is nowhere to put it. **Say so** — see [`deposit`].
    NoBase,
    /// The registry does not know this id, which should be impossible and is
    /// worth reporting rather than dropping: it would surface later as a
    /// conservation mismatch with no explanation.
    Unknown,
}

/// Bank a broken block on the fleet's pile, and name what happened.
///
/// Everything you break is stock: every block yields itself by name onto the
/// same pile the drones haul into and the shop sells out of.
///
/// The `NoBase` arm is stage 48's bug fix. There is no base at boot — the pile
/// only exists once a container has been placed to declare it — and this was
/// written as a bare `if let Some(base)` with no `else`, so a new player who
/// walked out and mined got **nothing, silently**: the block broke, the ore
/// evaporated, and no line was printed anywhere. Every sibling system in the
/// game says so out loud ("NO BASE PILE. PLACE A CONTAINER.", "NO BASE PILE TO
/// DRAW ON", "NO BASE CONTAINER SET"); the one that produces the goods was the
/// only one that stayed quiet. Returning the outcome rather than a `bool` is
/// what lets the caller say it.
pub fn deposit(
    base: Option<&mut Stockpile>,
    registry: &BlockRegistry,
    block: BlockId,
    crate_here: bool,
) -> Deposited {
    if crate_here {
        return Deposited::Crate;
    }
    let Some(pile) = base else {
        return Deposited::NoBase;
    };
    match registry.get(block) {
        Some(def) => {
            let name = def.name.clone();
            pile.add(name.clone(), 1);
            Deposited::Piled(name)
        }
        None => Deposited::Unknown,
    }
}

/// The line the player sees when the ore has nowhere to go.
///
/// Worded to match the printer's and the job board's, because it is the same
/// sentence about the same missing thing and three phrasings of one rule is
/// how a player learns to read past all of them.
pub const NO_BASE: &str = "NO BASE PILE. PLACE A CONTAINER.";

#[cfg(test)]
mod tests {
    use super::*;
    /// The real block table, the way every other test module gets one.
    fn world() -> vx_world::World {
        vx_world::World::new(1)
    }

    /// The number the opening is paced by. Copper is 2.5 hard, a drill nobody
    /// has spent anything on is 1.25 a second, so an ore block is two seconds
    /// of holding still on the day you arrive.
    #[test]
    fn copper_takes_two_seconds_on_the_drill_you_start_with() {
        let power = power_of(1, 0);
        assert!((power - 1.25).abs() < 1e-6, "a fresh drill is {power}");
        let seconds = seconds_for(2.5, power);
        assert!((seconds - 2.0).abs() < 1e-6, "copper took {seconds}s");

        // And the bite is the inverse: holding for exactly that long is
        // exactly one block.
        let step = bite(2.5, power, seconds);
        assert!((step - 1.0).abs() < 1e-6, "a full hold moved {step}");
    }

    /// Skills and money multiply rather than add, and both make it faster.
    #[test]
    fn levels_and_upgrades_both_speed_the_bit_up() {
        let plain = power_of(1, 0);
        assert!(power_of(10, 0) > plain, "levelling did nothing");
        assert!(power_of(1, 1) > plain, "the upgrade did nothing");
        // 25% for the first mark of the line, exactly.
        assert!((power_of(1, 1) - plain * 1.25).abs() < 1e-6);
    }

    /// The bit skates on bedrock rather than dividing by nothing. Hardness of
    /// zero is not a fast block, it is a block with no hardness at all — and
    /// the caller refuses those before it gets here — but the floor has to
    /// hold anyway, because a *nearly* free block would otherwise finish in
    /// one frame with a visible pop.
    #[test]
    fn a_nearly_free_block_still_takes_a_frame_rather_than_none() {
        let step = bite(0.0, power_of(1, 0), 1.0 / 60.0);
        assert!(step.is_finite(), "the bite went to infinity");
        assert!(step > 0.0);
    }

    /// A worked face shows its work: four layers, one per quarter, and never
    /// on the frame the block finishes — that frame draws the break instead.
    #[test]
    fn a_hold_carves_four_layers_and_none_on_the_last_frame() {
        let step = 0.1;
        let mut progress = None;
        let mut carves = 0;
        let mut frames = 0;
        loop {
            let outcome = advance_bite(progress, step);
            frames += 1;
            if outcome.carve {
                carves += 1;
            }
            if outcome.through {
                assert!(!outcome.carve, "carved a layer on the breaking frame");
                break;
            }
            progress = Some(outcome.progress);
            assert!(frames < 100, "the hold never finished");
        }
        // Quarters at 0.25, 0.50 and 0.75 are crossed while still short of
        // the break; the fourth quarter *is* the break.
        assert_eq!(carves, 3, "a worked face showed {carves} layers");
    }

    /// Look away and you start again. This is the rule that makes the lockbox
    /// exception mean something.
    #[test]
    fn moving_the_aim_starts_the_block_over() {
        let carried = advance_bite(None, 0.4);
        assert!((carried.progress - 0.4).abs() < 1e-6);
        let kept = advance_bite(Some(carried.progress), 0.4);
        assert!((kept.progress - 0.8).abs() < 1e-6, "the hold did not carry");
        // A fresh target ignores everything that came before it.
        let restarted = advance_bite(None, 0.4);
        assert!((restarted.progress - 0.4).abs() < 1e-6, "the aim did not reset");
    }

    /// One enormous frame cannot carry a brand new block past the break
    /// without ever drawing it. A continuing hold may overshoot; a fresh one
    /// is clamped.
    #[test]
    fn a_stalled_frame_cannot_skip_a_fresh_block() {
        let huge = advance_bite(None, 50.0);
        assert_eq!(huge.progress, 1.0, "a fresh block overshot to {}", huge.progress);
        assert!(huge.through);
    }

    /// The whole point of the round: an ore block with nowhere to go says so,
    /// rather than evaporating.
    #[test]
    fn mining_with_no_base_reports_it_instead_of_eating_the_block() {
        let world = world();
        let registry = world.registry();
        let ore = registry.id_of("engine:copper_ore").expect("no copper ore");

        assert_eq!(
            deposit(None, registry, ore, false),
            Deposited::NoBase,
            "a block mined with no pile went quietly"
        );

        let mut pile = Stockpile::new();
        assert_eq!(
            deposit(Some(&mut pile), registry, ore, false),
            Deposited::Piled("engine:copper_ore".into())
        );
        assert_eq!(pile.count("engine:copper_ore"), 1);
    }

    /// A crate is prised open, not harvested: it pays out its haul elsewhere
    /// and yields no crate-shaped block of its own.
    #[test]
    fn a_supply_cache_yields_its_haul_and_not_itself() {
        let world = world();
        let registry = world.registry();
        let cache = registry.id_of("engine:supply_cache").expect("no cache block");
        let mut pile = Stockpile::new();
        assert_eq!(deposit(Some(&mut pile), registry, cache, true), Deposited::Crate);
        assert_eq!(pile.total(), 0, "the crate itself landed on the pile");
    }

    /// Every face the raycast can report has an index the carve shapes accept.
    #[test]
    fn every_face_has_an_index() {
        for (expected, face) in Face::ALL.iter().enumerate() {
            assert_eq!(face_index(*face), expected);
        }
    }

    /// The line has to be one the bitmap font can actually draw, like every
    /// other string that reaches the screen.
    #[test]
    fn the_missing_pile_line_is_drawable() {
        for character in NO_BASE.chars() {
            assert!(
                vx_render::font::knows(character),
                "the font cannot draw {character:?} in the no-base line"
            );
        }
    }
}
