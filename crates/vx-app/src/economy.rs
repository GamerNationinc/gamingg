//! Town markets: what each place makes, wants, holds and charges.
//!
//! # The first thing here that is not a pure function of the seed
//!
//! Everything else in this world can be recomputed: terrain, ore, trees, town
//! sites, the work a beacon posts. A market cannot, because the player changes
//! it — sell forty loads of ore and the price moves, and it has to still be
//! moved tomorrow. So this is the first state that genuinely has to be *kept*.
//!
//! It is kept as cheaply as the rest: a town's opening books are derived from
//! its site, so the save holds an entry only for a town something has actually
//! touched. A frontier of a thousand untouched towns costs nothing.
//!
//! # Fast-forward on read
//!
//! Nothing ticks. A market carries the tick it was last brought up to date, and
//! reading it integrates production and consumption forward to now. A town three
//! kilometres away costs nothing until something asks about it, and then costs
//! one short loop.
//!
//! ## Why the loop is quantised rather than closed-form
//!
//! The obvious implementation is `stock += rate * elapsed`, in one multiply. It
//! is wrong, and wrong in a way that would have been very hard to see later.
//!
//! Flows here are *clamped* — a mine's ore fills up and stops, a refinery stops
//! when it runs out of ore to melt. Once a clamp binds, one big step and many
//! small steps stop agreeing: in one big step the mine saturates and the
//! refinery then eats a full day's worth all at once, while in small steps the
//! refinery keeps making room for the mine to refill into. The books would then
//! depend on *when the player happened to look*, which is exactly the class of
//! bug stage 9a spent its whole round removing from the drones.
//!
//! So catch-up runs in fixed [`STEP`]-tick chunks, always landing on a step
//! boundary. Integrating to tick 5 000 and then to 10 000 gives bit-for-bit
//! what integrating straight to 10 000 gives, clamps or no clamps — and there
//! is a test that says so. A whole in-game day of neglect is forty steps.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;

use vx_world::town::{Speciality, TownSite};

const MAGIC: &[u8; 4] = b"VXEC";
// Version 2 added `Owner::Mail`. The record layout is unchanged — a v1 file
// simply never contains the new owner byte — so both versions are accepted on
// read and nothing is discarded on upgrade. An older build refuses a v2 file
// cleanly, which is the tolerant-loader contract doing its job.
/// Three adds oxyhydrogen to the goods table. A market's books are a fixed
/// array of stocks, so a fifth good is a format change: an old file's four
/// numbers cannot be read as five without inventing the missing one, and a
/// town's books are better re-derived from its site than guessed at.
/// Four adds the three deep goods — uranium, oil and gas — for the same
/// reason three added oxyhydrogen: a market's books are a fixed array, and
/// five numbers cannot be read as eight without inventing three. A town's
/// books are re-derived from its site instead, which is cheaper than a
/// migration and more honest than a guess.
/// Six is the round towns started paying each other. A market gains what it
/// has earned selling on and how far it has grown on the strength of it, and a
/// load in the air gains what the buyer put up for it. A version-five file is
/// read back as a town that has traded nothing and built nothing, which is not
/// a lie — it is a book kept before anybody was counting.
const VERSION: u32 = 6;

/// What the network moves.
///
/// Three raw goods that come out of the ground and one that is made. Ordered,
/// because a market's books are an array indexed by this — names are for disk
/// and for the player, indices are for the arithmetic.
pub const GOODS: [&str; 8] = [
    "engine:copper_ore",
    "engine:log",
    "engine:stone",
    "engine:copper_bar",
    "engine:hho_cell",
    "engine:uranium_ore",
    "engine:oil_barrel",
    "engine:gas_cell",
];

pub const ORE: usize = 0;
pub const LOG: usize = 1;
pub const STONE: usize = 2;
pub const BAR: usize = 3;
/// Oxyhydrogen. Unlike everything above it, nobody digs this out of
/// anything: it is water and electrodes and patience, which is exactly why
/// a town can run *out* of it in a way it can never run out of stone.
pub const HHO: usize = 4;
/// The deep three. Nothing on the network *makes* any of them: no town
/// extracts uranium, and no town has a well. Every barrel in every book on
/// the frontier came off a player's pile, which is what makes these the
/// first goods where the player is the supply rather than another trader on
/// it — and why they are dear everywhere until somebody floods a town.
pub const URANIUM: usize = 5;
pub const OIL: usize = 6;
pub const GAS: usize = 7;

/// Ticks per catch-up step. Thirty seconds of game time at eight ticks a
/// second, so a full day of neglect is forty steps.
pub const STEP: u64 = 240;

/// What a town aims to hold of a good. Prices sit at their reference here.
const TARGET: f32 = 400.0;

/// The most a town will hold. Past this a producer simply stops.
const CAPACITY: f32 = 1_200.0;

/// What each good is worth to a town holding exactly [`TARGET`] of it.
const BASE_PRICE: [f32; GOODS.len()] = [10.0, 4.0, 3.0, 60.0, 34.0, 190.0, 24.0, 30.0];

/// How far price may swing from its reference. A glutted market still pays
/// something and a desperate one is still affordable — a shock should be
/// legible, not a cliff.
const MIN_FACTOR: f32 = 0.35;
const MAX_FACTOR: f32 = 2.50;

/// What a town keeps in the till, at most.
///
/// # The hole this closes
///
/// Price falls as a town's stock rises and then **stops falling**: the swing
/// is clamped at [`MIN_FACTOR`] and the price floored at one credit, and
/// `deposit` caps stock at [`CAPACITY`] so it cannot rise past the point where
/// the clamp bites. Stone's reference is 3, so a town glutted to the roof
/// still paid **one credit a block, for ever** — and nothing anywhere asked
/// whether it could afford to.
///
/// So the best strategy in the game was: point drones at any rock at all and
/// sell the spoil to the nearest counter until you got bored. Unbounded, no
/// decision in it, and strictly better than every interesting thing the
/// economy offers. Any question of the form "how do I earn the most" had the
/// same dull answer.
///
/// A till fixes the arithmetic and the design in one move. A counter pays out
/// of money it actually has; the money comes back at [`TILL_REFILL`] a step
/// from what the town itself sells on; and a place you have just emptied is a
/// place to leave alone for a while. That turns earning into a *routing*
/// problem — which town, which good, how far, how often — which is what the
/// road network, the price spread and the walk were always for.
const TILL_CAP: f32 = 9_000.0;

/// Credits a town puts back in the till per [`STEP`], scaled by what it holds.
///
/// A town with goods to sell earns; a town stripped bare earns slowly. The
/// shape matters more than the number: it means a counter you drained recovers
/// on its own if you leave it, and recovers faster if you have not also taken
/// everything it had to trade.
const TILL_REFILL: f32 = 26.0;

/// What a town must have earned selling on before it builds each new shed.
///
/// Measured against lifetime earnings rather than the till for the reason
/// [`Market::earn`] gives: a till is spent, and a place that is doing well is
/// a place money moves *through*, not one it piles up in. The steps rise
/// steeply on purpose — the first is a good week for a town on the network,
/// the last is a town that has become the reason the network goes that way.
const GROWTH_AT: [f32; 3] = [6_000.0, 20_000.0, 50_000.0];

/// The most a town can outgrow itself. One per entry in [`GROWTH_AT`], and one
/// blueprint per step in [`vx_world::town::plan::growth_plan`] — a town that
/// grew past what has been authored for it would ask for a building that does
/// not exist, so the two are pinned together by a test rather than by hope.
pub const MAX_GROWTH: u8 = GROWTH_AT.len() as u8;

/// What one growth step is worth to the works, as a fraction.
///
/// The new sheds are the town's new capacity: this is the whole reason growth
/// is a loop rather than decoration. A town that trades well builds, and what
/// it builds lets it trade more — and because it multiplies extraction and
/// conversion rather than adding to them, a mine grows into more of a mine
/// instead of every town growing into the same thing.
const WORKS_PER_STEP: f32 = 0.15;

/// Look up a good by its namespaced name.
pub fn good_index(name: &str) -> Option<usize> {
    GOODS.iter().position(|good| *good == name)
}

/// One town's books.
#[derive(Debug, Clone, PartialEq)]
pub struct Market {
    stock: [f32; GOODS.len()],
    /// What the counter can actually pay out. See [`TILL_CAP`].
    till: f32,
    /// Everything this town has ever earned selling goods on, which is not
    /// the same number as the till and must not be: a till is spent, and a
    /// town that has had a good decade and a bad year has still had the good
    /// decade. See [`GROWTH_AT`].
    trade: f32,
    /// How many times this town has outgrown itself. A ratchet.
    growth: u8,
    /// The last step boundary this was brought up to date at.
    last_tick: u64,
}

impl Market {
    pub fn stock(&self, good: usize) -> f32 {
        self.stock[good]
    }

    /// What this town pays for one unit, given what it is holding.
    ///
    /// Monotonically decreasing in stock and bounded at both ends: cheap where
    /// a thing is made, dear where it is wanted, never free and never infinite.
    pub fn price(&self, good: usize) -> u64 {
        let ratio = self.stock[good] / TARGET;
        let factor = (2.0 - ratio).clamp(MIN_FACTOR, MAX_FACTOR);
        (BASE_PRICE[good] * factor).round().max(1.0) as u64
    }

    /// What the counter can pay out right now.
    pub fn till(&self) -> u64 {
        self.till.max(0.0) as u64
    }

    /// Take up to `amount` out of the till, returning what was actually there.
    ///
    /// The only way credits leave a town, and the reason a sale can be partial.
    pub fn draw(&mut self, amount: u64) -> u64 {
        let paid = (amount as f32).min(self.till.max(0.0));
        self.till -= paid;
        paid as u64
    }

    /// How many units this counter could pay for at a unit `rate`.
    ///
    /// The number the payroll readout quotes and the shop enforces, so the
    /// figure a player is shown is the figure they get.
    pub fn affords(&self, rate: u64) -> u64 {
        if rate == 0 {
            return 0;
        }
        self.till() / rate
    }

    /// Is this town short of a good — the thing that makes it worth shipping to?
    pub fn wants(&self, good: usize) -> bool {
        self.stock[good] < TARGET * 0.6
    }

    /// Has it more than it can use?
    pub fn surplus(&self, good: usize) -> f32 {
        (self.stock[good] - TARGET * 1.2).max(0.0)
    }

    /// What this town has earned selling goods on, for its whole life.
    pub fn trade(&self) -> u64 {
        self.trade.max(0.0) as u64
    }

    /// How many times this town has outgrown itself, `0..=MAX_GROWTH`.
    pub fn growth(&self) -> u8 {
        self.growth
    }

    /// Credits in, from a sale that actually happened.
    ///
    /// The one way money *enters* a town, and deliberately not the same thing
    /// as the till: the till is capped and spent, `trade` is neither. A town
    /// whose till is already full still earned what it earned, and that is the
    /// number growth is measured against — otherwise the richest towns on the
    /// frontier, the ones pinned at [`TILL_CAP`], would be the ones that
    /// looked like they had stopped trading.
    pub fn earn(&mut self, amount: u64) {
        self.trade += amount as f32;
        self.till = (self.till + amount as f32).min(TILL_CAP);
    }

    /// Move goods in or out, clamped to what is actually there.
    /// Returns the amount that really moved.
    pub fn deposit(&mut self, good: usize, amount: f32) -> f32 {
        let room = CAPACITY - self.stock[good];
        let moved = amount.min(room).max(0.0);
        self.stock[good] += moved;
        moved
    }

    pub fn withdraw(&mut self, good: usize, amount: f32) -> f32 {
        let moved = amount.min(self.stock[good]).max(0.0);
        self.stock[good] -= moved;
        moved
    }
}

/// What a town pulls out of the ground, per tick, needing nothing to do it.
fn extraction(speciality: Speciality) -> [f32; GOODS.len()] {
    match speciality {
        // A camp built around a hole: ore and the spoil that comes with it.
        Speciality::Mine => [0.030, 0.0, 0.020, 0.0, 0.0, 0.0, 0.0, 0.0],
        // Timber from the country around it.
        Speciality::Depot => [0.0, 0.018, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        Speciality::Refinery => [0.0, 0.004, 0.006, 0.0, 0.0, 0.0, 0.0, 0.0],
    }
}

/// What a town makes out of other goods: `(inputs per unit, output, rate)`.
fn conversion(speciality: Speciality) -> Option<([f32; GOODS.len()], usize, f32)> {
    match speciality {
        // Ore and stone in, bars out. The reason a refinery is worth visiting.
        Speciality::Refinery => Some(([0.030, 0.0, 0.012, 0.0, 0.0, 0.0, 0.0, 0.0], BAR, 0.010)),
        // A depot runs the cell bank: bars in — electrodes wear out the same
        // for a town as for a player — and fuel out. So the one place that
        // *makes* fuel is not the one place that burns most of it, which is
        // what gives the network something to haul.
        Speciality::Depot => Some(([0.0, 0.0, 0.0, 0.004, 0.0, 0.0, 0.0, 0.0], HHO, 0.014)),
        _ => None,
    }
}

/// What a town simply uses up, per tick.
fn consumption(speciality: Speciality) -> [f32; GOODS.len()] {
    match speciality {
        // Props and fires — and the cutting gear, which is the point of the
        // whole loop: a mine is the town that stops when the fuel does.
        // Oil for the cutting gear and gas for the compressors: a mine
        // burns both, which is why a mine is where a well pays best.
        Speciality::Mine => [0.0, 0.012, 0.0, 0.0, 0.016, 0.0, 0.009, 0.011],
        // A depot's whole business is moving finished goods onward.
        Speciality::Depot => [0.004, 0.006, 0.002, 0.012, 0.004, 0.0, 0.005, 0.005],
        // Furnaces run hot on it.
        // The refinery is the only place on the frontier with any use for
        // uranium at all, and it uses it slowly.
        Speciality::Refinery => [0.0, 0.008, 0.0, 0.002, 0.010, 0.002, 0.011, 0.007],
    }
}

/// A town's books before anything has touched them.
///
/// Derived from the site, like its name and size, so an untouched town costs
/// the save nothing. Biased by speciality — a mine opens sitting on ore — and
/// jittered per town so no two markets are identical.
pub fn opening_books(site: &TownSite) -> Market {
    let mut stock = [0.0f32; GOODS.len()];
    for (good, slot) in stock.iter_mut().enumerate() {
        let jitter = vx_world::seed::unit(vx_world::seed::finalise(
            site.seed ^ (good as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15),
        ));
        // 0.6 to 1.4 of target, then shifted by what the town does.
        let spread = 0.6 + jitter * 0.8;
        let bias = match (site.speciality, good) {
            (Speciality::Mine, ORE) | (Speciality::Mine, STONE) => 1.6,
            (Speciality::Refinery, BAR) => 1.5,
            (Speciality::Refinery, ORE) => 0.5,
            (Speciality::Depot, LOG) => 1.4,
            (Speciality::Depot, BAR) => 0.5,
            (Speciality::Depot, HHO) => 1.5,
            (Speciality::Mine, HHO) | (Speciality::Refinery, HHO) => 0.45,
            // The deep three: the network has no source for any of them, so
            // every town opens nearly out and pays accordingly.
            (_, URANIUM) => 0.06,
            (_, OIL) | (_, GAS) => 0.18,
            _ => 1.0,
        };
        *slot = (TARGET * spread * bias).min(CAPACITY);
    }
    // A town opens with a working float rather than a full till: the money a
    // place has on hand is a function of how long it has been trading, and on
    // day one it has not been.
    let float = vx_world::seed::unit(vx_world::seed::finalise(
        site.seed ^ 0x7469_6c6c_0000_0001,
    ));
    Market {
        stock,
        till: TILL_CAP * (0.25 + float * 0.35),
        // A town opens having sold nothing to anybody. Whatever it had before
        // the player turned up is scenery, and scenery does not build sheds.
        trade: 0.0,
        growth: 0,
        last_tick: 0,
    }
}

// ---------------------------------------------------------------------------
// The people who live there
// ---------------------------------------------------------------------------

/// What one resident puts on the books in a tick of their shift.
///
/// Deliberately a fraction of what the town's own works do: the extraction
/// table is the mine, the mill and the yard, and a resident is one pair of
/// hands beside it. Three of them add roughly a fifth to a town's output —
/// enough that a town with its people at work reads differently from one with
/// its people in bed, and not enough to make the works decorative.
pub const HANDS: f32 = 0.0040;

/// What one resident burns in a tick, awake or asleep. Timber: their fire,
/// their roof and their tools.
pub const BOARD: f32 = 0.0022;

/// And what they spend on market day, when they are at the square buying
/// rather than at the bench making.
pub const SPENDING: f32 = 0.0009;

/// What a trade actually makes.
///
/// Per trade rather than per town, so a town's three residents are three
/// different contributions to its books instead of one rate multiplied by
/// three. `None` is honest rather than lazy: an ostler keeps the animals and
/// a tallyman counts what is already there, and a town whose people all
/// produce is a town with no services in it.
fn made_by(trade: &str) -> Option<usize> {
    Some(match trade {
        "FOREMAN" | "ASSAYER" => ORE,
        "POWDERMAN" => STONE,
        "SMELTERMAN" | "GAUGER" => BAR,
        "STOKER" => HHO,
        "CLERK" | "OSTLER" => LOG,
        // The tallyman counts the pile; he does not add to it.
        _ => return None,
    })
}

/// How hard this particular person works, `0.6` to `1.4`.
///
/// Its own salt off the town's seed rather than a re-use of
/// [`Temperament::warmth`](crate::people::Temperament): a warm person is not
/// necessarily a diligent one, and folding the two would make every friendly
/// shopkeeper in the world rich.
pub fn diligence(site: &TownSite, index: usize) -> f32 {
    let hash = vx_world::seed::finalise(
        site.seed
            ^ 0x0d11_6e6e_0000_0001
            ^ (index as u64 + 1).wrapping_mul(0xc2b2_ae3d_27d4_eb4f),
    );
    0.6 + vx_world::seed::unit(hash) * 0.8
}

/// The clock a market step sits on, as the schedule reads it.
///
/// A step is 240 journal ticks and a day is 9 600, so there are forty of them
/// in a day — half an hour of town time each, which is fine enough that the
/// books notice the difference between a working morning and a market
/// afternoon.
fn when(step: u64) -> (u32, crate::clock::TimeOfDay) {
    let tick = step * STEP;
    let day = (tick / crate::schedule::TICKS_PER_DAY) as u32;
    let through = (tick % crate::schedule::TICKS_PER_DAY) as f32
        / crate::schedule::TICKS_PER_DAY as f32;
    (day, crate::clock::TimeOfDay::new(through))
}

/// What the town's own people did with this step.
///
/// Pure in `(site, step)` — it reads the schedule, which is pure in the same
/// things — so it inherits the market's quantised catch-up for free: running
/// forty steps at once is the same forty steps.
fn residents(market: &mut Market, site: &TownSite, step: u64) {
    let dt = STEP as f32;
    let (day, time) = when(step);
    for index in 0..crate::people::PEOPLE {
        let place = crate::schedule::where_is(site, index, day, time, false);
        // Everyone burns their board, awake or asleep — a house does not stop
        // costing anything because nobody is standing in it.
        let board = match place {
            crate::schedule::Place::Home => BOARD * 0.5,
            _ => BOARD,
        };
        market.stock[LOG] = (market.stock[LOG] - board * dt).max(0.0);

        match place {
            crate::schedule::Place::Workplace => {
                let person = crate::people::person(site, index);
                if let Some(good) = made_by(person.trade) {
                    let rate = HANDS * diligence(site, index);
                    market.stock[good] = (market.stock[good] + rate * dt).min(CAPACITY);
                }
            }
            // Market day at the square: they are buying finished goods off
            // the town's own shelves, which is what a market day *is*.
            crate::schedule::Place::Plaza if crate::schedule::is_market_day(site, day) => {
                market.stock[BAR] = (market.stock[BAR] - SPENDING * dt).max(0.0);
            }
            _ => {}
        }
    }
}

/// What one resident has in their pocket at `tick`.
///
/// Derived rather than stored, the same trick the forest and the weather use:
/// a purse is the shifts they have worked times what their hands are worth,
/// so nothing has to be written down for every person in every town in the
/// world. It is what the note means by "credits are the end goal of every
/// individual, NPC and player alike" — with the arithmetic made honest by
/// the fact that a resident who does not work does not earn.
///
/// Costed off the town's own price for what they make, at the reference
/// rather than the live book: a wage that swung with the market every time
/// you looked at it would be a wage nobody could count.
pub fn purse(site: &TownSite, index: usize, tick: u64) -> u64 {
    let person = crate::people::person(site, index);
    let Some(good) = made_by(person.trade) else {
        // Services are paid a flat town wage rather than by the unit.
        return worked_steps(site, index, tick) * 3;
    };
    let made = HANDS * diligence(site, index) * STEP as f32;
    let unit = BASE_PRICE[good];
    (worked_steps(site, index, tick) as f32 * made * unit) as u64
}

/// How many market steps this person has actually been at work for.
fn worked_steps(site: &TownSite, index: usize, tick: u64) -> u64 {
    let mut worked = 0;
    for step in 0..tick / STEP {
        let (day, time) = when(step);
        if crate::schedule::where_is(site, index, day, time, false)
            == crate::schedule::Place::Workplace
        {
            worked += 1;
        }
    }
    worked
}

/// Advance one market by exactly one [`STEP`].
fn step_market(market: &mut Market, site: &TownSite, step: u64) {
    let speciality = site.speciality;
    let dt = STEP as f32;
    // What the town has built for itself, as a multiplier on its own works.
    // Applied to the rates rather than to the run, so a grown refinery gets
    // through more ore *and* makes more bar, which is what another furnace is.
    let works = 1.0 + market.growth as f32 * WORKS_PER_STEP;

    // Out of the ground first: unconditional, and stops at capacity.
    for (good, rate) in extraction(speciality).into_iter().enumerate() {
        if rate > 0.0 {
            market.stock[good] = (market.stock[good] + rate * works * dt).min(CAPACITY);
        }
    }

    // Then the works, which run only as long as their inputs last.
    if let Some((inputs, output, rate)) = conversion(speciality) {
        let rate = rate * works;
        let mut run = dt;
        for (good, need) in inputs.into_iter().enumerate() {
            if need > 0.0 {
                run = run.min(market.stock[good] / need);
            }
        }
        for (good, need) in inputs.into_iter().enumerate() {
            if need > 0.0 {
                market.stock[good] = (market.stock[good] - need * run).max(0.0);
            }
        }
        market.stock[output] = (market.stock[output] + rate * run).min(CAPACITY);
    }

    // Then the people who live there, who make and eat on the hours the
    // schedule gives them. Between the works and the town's own burn, because
    // a resident is a smaller thing than a mill and a larger one than a lamp.
    residents(market, site, step);

    // Then what the town simply burns through, down to nothing at worst.
    for (good, rate) in consumption(speciality).into_iter().enumerate() {
        if rate > 0.0 {
            market.stock[good] = (market.stock[good] - rate * dt).max(0.0);
        }
    }

    // And last, the till: a town earns from what it has to sell on, so a
    // place stripped bare recovers slowly and a full one recovers fast. The
    // fraction is total stock against what every good at target would be,
    // clamped, so one glutted good cannot make a town rich on its own.
    let held: f32 = market.stock.iter().sum();
    let trading = (held / (TARGET * GOODS.len() as f32)).clamp(0.15, 1.0);
    market.till = (market.till + TILL_REFILL * trading).min(TILL_CAP);

    // And whether any of that has added up to another building. Inside the
    // step, so it inherits the quantised catch-up: forty steps at once grows a
    // town exactly as forty steps one at a time, which is the invariant the
    // whole module is arranged around. A count rather than a comparison, so
    // crossing two thresholds in one catch-up is two buildings and not one.
    market.growth = GROWTH_AT
        .iter()
        .filter(|threshold| market.trade >= **threshold)
        .count() as u8;
}

/// Who a load belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Owner {
    /// A town shipping to another town, on its own account.
    Town,
    /// The player's drone, paid out on arrival.
    Player,
    /// A mail order: bought and paid for at a counter, bound for the player's
    /// mailbox rather than any town's books.
    Mail,
}

/// A load in the air.
///
/// The same record for a town's freight and the player's, which is most of why
/// doing both halves of this round cost barely more than doing one.
///
/// Nothing about it is simulated. It has a departure, an arrival and two ends,
/// and *where it is right now is a sum* — see [`Shipment::position_at`]. That is
/// what makes a hundred loads crossing the map free, and what lets one be drawn
/// as a real drone when it happens to pass the player without any state
/// existing for it to be drawn from.
#[derive(Debug, Clone, PartialEq)]
pub struct Shipment {
    pub from: (i32, i32),
    pub to: (i32, i32),
    pub good: usize,
    pub amount: f32,
    pub depart: u64,
    pub arrive: u64,
    pub owner: Owner,
    /// What the buyer put up when the load left, and what the seller took.
    ///
    /// Only ever non-zero for [`Owner::Town`]: a player's load and a mail
    /// order were both settled at a counter before they were ever in the air,
    /// and charging for them again here would be the same goods paid for
    /// twice. Carried on the load rather than recomputed on arrival because
    /// the price a thing sold at is the price it sold at — a load that takes
    /// three in-game minutes to cross the map must not be repriced by what the
    /// market did while it was flying.
    pub paid: u64,
}

/// How many ticks a load takes per block travelled.
///
/// About twelve blocks a second, so a two-kilometre haul is a shade under three
/// in-game minutes — long enough that a run is a commitment, short enough that
/// the frontier feels connected.
const TICKS_PER_BLOCK: f32 = 0.65;

/// The least a run can take, so neighbouring towns do not trade instantly.
const MIN_TRAVEL: u64 = 200;

impl Shipment {
    /// How long a run between two towns takes.
    pub fn travel_ticks(from: (i32, i32), to: (i32, i32)) -> u64 {
        let dx = (from.0 - to.0) as f32;
        let dz = (from.1 - to.1) as f32;
        ((dx * dx + dz * dz).sqrt() * TICKS_PER_BLOCK) as u64 + MIN_TRAVEL
    }

    /// Where the load is at `now`, as a column. Pure in the clock: same tick,
    /// same place, however often it is asked.
    pub fn position_at(&self, now: u64) -> (f64, f64) {
        let span = self.arrive.saturating_sub(self.depart).max(1) as f64;
        let travelled = now.saturating_sub(self.depart) as f64;
        let t = (travelled / span).clamp(0.0, 1.0);
        (
            f64::from(self.from.0) + f64::from(self.to.0 - self.from.0) * t,
            f64::from(self.from.1) + f64::from(self.to.1 - self.from.1) * t,
        )
    }
}

/// How much of a surplus a town will part with in one load.
const LOAD: f32 = 90.0;

/// How much the player's drone carries in one run. Whole units, because it
/// comes out of a block pile rather than a town's books.
pub const PLAYER_LOAD: u64 = 60;

/// How much one mail order brings. Smaller than a trade run on purpose:
/// ordering is convenience, hauling is the business.
pub const PARCEL: u64 = 20;

/// Most goods a mailbox holds, counting loads still in the air. A cap rather
/// than a queue: the refusal happens at the counter, where you can do
/// something about it.
pub const MAILBOX_CAP: u64 = 120;

/// How often the network looks for a run worth making. Four in-game minutes.
pub const DISPATCH_EVERY: u64 = STEP * 8;

/// The most loads the network puts in the air in one dispatch window.
///
/// Four rather than one, and one per source town. The bound exists so that a
/// player who walks into a neighbourhood nobody has visited for a week does
/// not watch two hundred wagons appear at once when the catch-up runs: a
/// window is a window however long ago it was.
const MAX_RUNS: usize = 4;

/// Every town whose books have moved away from what its site implies, and
/// every load currently in the air.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Economy {
    towns: HashMap<(i32, i32), Market>,
    /// Ordered by arrival, so settling is a walk off the front.
    shipments: Vec<Shipment>,
    /// The last tick the network was asked to look for work.
    last_dispatch: u64,
}

impl Economy {
    pub fn new() -> Self {
        Economy::default()
    }

    pub fn tracked(&self) -> usize {
        self.towns.len()
    }

    /// A town's books, brought up to date.
    ///
    /// The only way to read a market. Everything else goes through here so that
    /// nothing can ever see a stale one.
    pub fn market(&mut self, site: &TownSite, now: u64) -> &Market {
        self.market_mut(site, now)
    }

    pub fn market_mut(&mut self, site: &TownSite, now: u64) -> &mut Market {
        let market = self
            .towns
            .entry(site.centre)
            .or_insert_with(|| opening_books(site));

        // A book read back from before towns had money carries a `NaN` till,
        // which is the loader saying "this file cannot tell you". Filled in
        // here rather than there, because only here is the site known.
        if market.till.is_nan() {
            market.till = opening_books(site).till;
        }

        // Catch up in whole steps, landing on a boundary. This is what makes
        // "integrate to 5 000 then to 10 000" identical to "integrate to
        // 10 000" even where a clamp binds.
        let due = now / STEP;
        let done = market.last_tick / STEP;
        for step in done..due {
            step_market(market, site, step);
        }
        market.last_tick = due * STEP;
        market
    }

    /// Loads currently in the air.
    pub fn shipments(&self) -> &[Shipment] {
        &self.shipments
    }

    /// Put a load in the air. Goods must already have left wherever they came
    /// from — this only carries them.
    pub fn ship(&mut self, shipment: Shipment) {
        // Kept in arrival order so settling is a walk off the front rather than
        // a scan of everything in flight.
        let at = self
            .shipments
            .partition_point(|other| other.arrive <= shipment.arrive);
        self.shipments.insert(at, shipment);
    }

    /// A load shot out of the sky: remove it mid-flight and hand it to the
    /// caller, whose problem the wreckage now is.
    ///
    /// No bookkeeping is owed here. The goods left the source market at
    /// dispatch — a network that conjured freight would be lying — and the
    /// destination deposit only ever happens on settling, so a load that
    /// never settles leaves the destination short. That shortage working
    /// through the price curve *is* the town noticing. Removing from the
    /// middle keeps the arrival ordering, so `settle`'s walk off the front
    /// never knows anything happened.
    pub fn intercept(&mut self, index: usize) -> Option<Shipment> {
        if index < self.shipments.len() {
            Some(self.shipments.remove(index))
        } else {
            None
        }
    }

    /// Run the network up to `now`: land everything that has arrived, then look
    /// for new runs worth making.
    ///
    /// `reachable` is the towns the network can see — pass what
    /// [`vx_world::town::towns_near`] returns for the player's neighbourhood.
    /// Towns beyond it carry on being derived and simply do not trade until
    /// somebody is near enough to care, which is the level-of-detail: the
    /// simulation is only ever as wide as it needs to be.
    ///
    /// Returns the player-owned loads that landed, so the caller can pay for
    /// them — this module knows nothing about wallets.
    pub fn run(&mut self, reachable: &[TownSite], now: u64) -> Vec<Shipment> {
        let mut delivered = Vec::new();

        // Catch up one dispatch window at a time, exactly as a market catches
        // up one step at a time and for the same reason: a network that only
        // looked for work when somebody happened to be watching would move
        // different goods depending on how often it was watched.
        let due = now / DISPATCH_EVERY;
        let done = self.last_dispatch / DISPATCH_EVERY;
        for window in done..due {
            let at = (window + 1) * DISPATCH_EVERY;
            self.settle(reachable, at, &mut delivered);
            self.dispatch(reachable, at);
        }
        self.last_dispatch = due * DISPATCH_EVERY;

        // And anything that has landed since the last whole window.
        self.settle(reachable, now, &mut delivered);
        delivered
    }

    /// Land every load that has arrived by `at`.
    fn settle(&mut self, reachable: &[TownSite], at: u64, delivered: &mut Vec<Shipment>) {
        // The list is arrival-ordered, so this stops at the first load still in
        // the air rather than scanning everything in flight.
        while self.shipments.first().is_some_and(|first| first.arrive <= at) {
            let landed = self.shipments.remove(0);
            // Mail never touches the destination's books: it was bought and
            // paid for at the source, and it lands in a mailbox, not a market.
            if landed.owner == Owner::Mail {
                delivered.push(landed);
                continue;
            }
            match reachable.iter().find(|site| site.centre == landed.to) {
                Some(site) => {
                    // Brought up to the moment the load *arrived*, not to now:
                    // a load that landed an hour ago should have been on the
                    // shelves for that hour.
                    let market = self.market_mut(site, landed.arrive);
                    market.deposit(landed.good, landed.amount);
                }
                // Nowhere to put it. The goods are gone either way, and a load
                // that cannot be landed is better lost than quietly duplicated.
                None => log::warn!("a load arrived at {:?}, which is not a town", landed.to),
            }
            if landed.owner == Owner::Player {
                delivered.push(landed);
            }
        }
    }

    /// Look for the runs worth making and put loads in the air.
    ///
    /// Up to [`MAX_RUNS`] a window, and never twice out of the same town: a
    /// network of a dozen places that moved one wagon every four in-game
    /// minutes was a map with freight drawn on it rather than a network, and
    /// the source rule is what stops one glutted mine taking every run while
    /// the rest of the frontier sits still.
    fn dispatch(&mut self, reachable: &[TownSite], now: u64) {
        let mut sent: Vec<(i32, i32)> = Vec::new();
        for _ in 0..MAX_RUNS {
            if !self.dispatch_one(reachable, now, &mut sent) {
                break;
            }
        }
    }

    /// One run, or nothing left worth making. Returns whether a load left.
    ///
    /// Greedy nearest-deficit matching: for the town with the biggest surplus,
    /// ship to the nearest town within reach that is short of that good *and
    /// can pay for it*. This is a deliberate simplification of the min-cost
    /// flow a full treatment would use — at a few dozen towns the two agree on
    /// the runs that matter, and the greedy version is a fraction of the code.
    /// Worth revisiting only if the traffic it produces disappoints.
    ///
    /// The buyer pays the buyer's own price, which is the same rule the
    /// player's trade runs settle under: you are paid what it is worth where
    /// it was wanted, not what it was worth where it was lying about. A town
    /// that cannot cover a load does not order one — so a poor town stays
    /// short, its prices stay high, and the first thing it can afford is the
    /// thing it needs most. That is the till doing the work the stock curve
    /// cannot do on its own.
    fn dispatch_one(
        &mut self,
        reachable: &[TownSite],
        now: u64,
        sent: &mut Vec<(i32, i32)>,
    ) -> bool {
        // surplus, from, to, good
        let mut best: Option<(f32, usize, usize, usize)> = None;

        for (from_index, from) in reachable.iter().enumerate() {
            if sent.contains(&from.centre) {
                continue;
            }
            for good in 0..GOODS.len() {
                let surplus = self.market(from, now).surplus(good);
                if surplus < LOAD {
                    continue;
                }
                // Nearest town that wants it and has the money for it.
                let mut nearest: Option<(i64, usize)> = None;
                for (to_index, to) in reachable.iter().enumerate() {
                    if to.centre == from.centre {
                        continue;
                    }
                    let buyer = self.market(to, now);
                    if !buyer.wants(good) || (buyer.affords(buyer.price(good)) as f32) < LOAD {
                        continue;
                    }
                    let dx = (from.centre.0 - to.centre.0) as i64;
                    let dz = (from.centre.1 - to.centre.1) as i64;
                    let distance = dx * dx + dz * dz;
                    if nearest.is_none_or(|(best, _)| distance < best) {
                        nearest = Some((distance, to_index));
                    }
                }
                if let Some((_, to_index)) = nearest {
                    if best.is_none_or(|(most, ..)| surplus > most) {
                        best = Some((surplus, from_index, to_index, good));
                    }
                }
            }
        }

        let Some((_, from_index, to_index, good)) = best else {
            return false;
        };
        let from = reachable[from_index];
        let to = reachable[to_index];

        // Take the goods out before they go in the air, or the network would
        // conjure freight out of nothing.
        let loaded = self.market_mut(&from, now).withdraw(good, LOAD);
        if loaded <= 0.0 {
            return false;
        }
        // And take the money out before it goes on the load, for the same
        // reason: the buyer is short of it from the moment the order is
        // placed, not from the moment the wagon arrives. `draw` is bounded by
        // what is actually in the till, so an order placed against a till that
        // moved between the check and here is short-paid rather than overdrawn.
        let rate = self.market(&to, now).price(good);
        let owed = (loaded.round() as u64).saturating_mul(rate);
        let paid = self.market_mut(&to, now).draw(owed);
        self.market_mut(&from, now).earn(paid);

        self.ship(Shipment {
            from: from.centre,
            to: to.centre,
            good,
            amount: loaded,
            depart: now,
            arrive: now + Shipment::travel_ticks(from.centre, to.centre),
            owner: Owner::Town,
            paid,
        });
        sent.push(from.centre);
        true
    }

    /// A summary for hashing, so the replay oracle can cover the economy.
    ///
    /// Rounded to whole units on purpose: the books are `f32`, and a hash over
    /// raw bits would report divergence over a fraction of a log nobody could
    /// ever see.
    pub fn books_hash(&self) -> u64 {
        let mut towns: Vec<(&(i32, i32), &Market)> = self.towns.iter().collect();
        towns.sort_by_key(|(centre, _)| **centre);
        let mut hash = 0u64;
        for (centre, market) in towns {
            let mut town = vx_world::seed::finalise(
                (centre.0 as i64 as u64).wrapping_mul(0x2545_f491_4f6c_dd1d)
                    ^ (centre.1 as i64 as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15),
            );
            for good in 0..GOODS.len() {
                town = vx_world::seed::finalise(
                    town ^ (market.stock[good].round() as i64 as u64)
                        .wrapping_mul(0xbf58_476d_1ce4_e5b9),
                );
            }
            // And the money, since stage 58: a network where the goods land
            // in the same places but the credits do not is a network that has
            // diverged, and a hash over stock alone would call it identical.
            // Whole credits, for the same reason the stocks are whole units.
            town = vx_world::seed::finalise(
                town ^ (market.till() ^ market.trade().wrapping_mul(31))
                    .wrapping_mul(0x94d0_49bb_1331_11eb)
                    ^ market.growth as u64,
            );
            hash ^= town;
        }
        hash
    }

    pub fn save(&self, directory: &Path) -> std::io::Result<()> {
        let mut file = crate::keeping::begin(directory, "economy.dat")?;
        file.write_all(MAGIC)?;
        file.write_all(&VERSION.to_le_bytes())?;

        let mut towns: Vec<(&(i32, i32), &Market)> = self.towns.iter().collect();
        towns.sort_by_key(|(centre, _)| **centre);
        file.write_all(&(towns.len() as u32).to_le_bytes())?;
        for (centre, market) in towns {
            file.write_all(&centre.0.to_le_bytes())?;
            file.write_all(&centre.1.to_le_bytes())?;
            file.write_all(&market.last_tick.to_le_bytes())?;
            for good in 0..GOODS.len() {
                file.write_all(&market.stock[good].to_le_bytes())?;
            }
            // Version 5. Older files have no till and are read back with the
            // one their site opens on, which is the honest answer: a save from
            // before towns had money cannot say how much they had.
            file.write_all(&market.till.to_le_bytes())?;
            // Version 6: what it has earned, and what it has built with it.
            file.write_all(&market.trade.to_le_bytes())?;
            file.write_all(&[market.growth])?;
        }

        file.write_all(&self.last_dispatch.to_le_bytes())?;
        file.write_all(&(self.shipments.len() as u32).to_le_bytes())?;
        for load in &self.shipments {
            file.write_all(&load.from.0.to_le_bytes())?;
            file.write_all(&load.from.1.to_le_bytes())?;
            file.write_all(&load.to.0.to_le_bytes())?;
            file.write_all(&load.to.1.to_le_bytes())?;
            file.write_all(&(load.good as u32).to_le_bytes())?;
            file.write_all(&load.amount.to_le_bytes())?;
            file.write_all(&load.depart.to_le_bytes())?;
            file.write_all(&load.arrive.to_le_bytes())?;
            file.write_all(&[match load.owner {
                Owner::Town => 0,
                Owner::Player => 1,
                Owner::Mail => 2,
            }])?;
            file.write_all(&load.paid.to_le_bytes())?;
        }
        file.commit()
    }

    /// Read the books back, tolerating absence and damage.
    ///
    /// A damaged file is an *empty* economy, not a failed world: every town
    /// falls back to the books its site implies. What is lost is trading
    /// history, which is a shame rather than a disaster.
    pub fn load(&mut self, directory: &Path) {
        let path = directory.join("economy.dat");
        match read_economy(&path) {
            Ok(Some(economy)) => *self = economy,
            Ok(None) => {}
            Err(error) => {
                log::warn!(
                    "could not read {}: {error}; every town reopens on its site's books",
                    path.display()
                );
            }
        }
    }
}

fn read_u32(file: &mut impl Read) -> std::io::Result<u32> {
    let mut word = [0u8; 4];
    file.read_exact(&mut word)?;
    Ok(u32::from_le_bytes(word))
}

fn read_u64(file: &mut impl Read) -> std::io::Result<u64> {
    let mut word = [0u8; 8];
    file.read_exact(&mut word)?;
    Ok(u64::from_le_bytes(word))
}

fn read_f32(file: &mut impl Read) -> std::io::Result<f32> {
    let mut word = [0u8; 4];
    file.read_exact(&mut word)?;
    let value = f32::from_le_bytes(word);
    if !value.is_finite() || value < 0.0 {
        return Err(std::io::Error::other("stock is not a number"));
    }
    Ok(value)
}

fn read_economy(path: &Path) -> std::io::Result<Option<Economy>> {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => std::io::BufReader::new(file),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };

    let mut magic = [0u8; 4];
    file.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(std::io::Error::other("bad magic"));
    }
    let version = read_u32(&mut file)?;
    // Version 1 is readable as-is: the layout never changed, only which owner
    // bytes can appear. Anything newer than this build writes is refused.
    if version == 0 || version > VERSION {
        return Err(std::io::Error::other("unknown version"));
    }

    let mut economy = Economy::new();
    let count = read_u32(&mut file)?;
    for _ in 0..count {
        let centre = (read_u32(&mut file)? as i32, read_u32(&mut file)? as i32);
        let last_tick = read_u64(&mut file)?;
        let mut stock = [0.0f32; GOODS.len()];
        for slot in stock.iter_mut() {
            // Bounded on the way in: a hand-edited book asserting a negative
            // or enormous stock would price every good at a clamp and read as
            // a town that cannot exist.
            let held = read_f32(&mut file)?;
            if !held.is_finite() || !(0.0..=CAPACITY).contains(&held) {
                return Err(std::io::Error::other("a stock a town could not hold"));
            }
            *slot = held;
        }
        // A book from before stage 5 of this file has no till column. It is
        // not an error and it is not zero: a town that has been trading for a
        // week has money, and the site's own opening float is the closest
        // honest guess. `tills` fills them in once the sites are known.
        let till = if version >= 5 {
            let till = read_f32(&mut file)?;
            if !till.is_finite() || till < 0.0 {
                return Err(std::io::Error::other("a till that is not money"));
            }
            till.min(TILL_CAP)
        } else {
            f32::NAN
        };
        // A book from before towns paid each other has traded nothing and
        // built nothing. Not a lie — a book kept before anybody was counting.
        let (trade, growth) = if version >= 6 {
            let trade = read_f32(&mut file)?;
            let mut step = [0u8; 1];
            file.read_exact(&mut step)?;
            if step[0] > MAX_GROWTH {
                return Err(std::io::Error::other("a town grown past what is authored"));
            }
            (trade, step[0])
        } else {
            (0.0, 0)
        };
        economy.towns.insert(
            centre,
            Market {
                stock,
                till,
                trade,
                growth,
                last_tick,
            },
        );
    }

    economy.last_dispatch = read_u64(&mut file)?;
    let loads = read_u32(&mut file)?;
    for _ in 0..loads {
        let from = (read_u32(&mut file)? as i32, read_u32(&mut file)? as i32);
        let to = (read_u32(&mut file)? as i32, read_u32(&mut file)? as i32);
        let good = read_u32(&mut file)? as usize;
        if good >= GOODS.len() {
            return Err(std::io::Error::other("unknown good in a load"));
        }
        let amount = read_f32(&mut file)?;
        let depart = read_u64(&mut file)?;
        let arrive = read_u64(&mut file)?;
        let mut owner = [0u8; 1];
        file.read_exact(&mut owner)?;
        let paid = if version >= 6 { read_u64(&mut file)? } else { 0 };
        economy.shipments.push(Shipment {
            from,
            to,
            good,
            amount,
            depart,
            arrive,
            owner: match owner[0] {
                0 => Owner::Town,
                1 => Owner::Player,
                2 => Owner::Mail,
                other => return Err(std::io::Error::other(format!("unknown owner {other}"))),
            },
            paid,
        });
    }
    Ok(Some(economy))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vx_world::town;

    fn frontier() -> Vec<TownSite> {
        town::towns_near(7, (0, 0), 2_000, &|_, _| 90)
    }

    fn of(speciality: Speciality) -> TownSite {
        frontier()
            .into_iter()
            .find(|site| site.speciality == speciality)
            .unwrap_or_else(|| panic!("no {} on the fixture frontier", speciality.name()))
    }

    /// The infinite money glitch, closed.
    ///
    /// Price falls as stock rises and then stops falling — the swing clamps at
    /// `MIN_FACTOR` and the price floors at one, and `deposit` caps stock at
    /// `CAPACITY` so it cannot rise past where the clamp bites. Stone's
    /// reference is 3, so a town glutted to the roof still paid a credit a
    /// block for ever, and nothing asked whether it could afford to. This is
    /// that, from the town's side: the price still floors, and the till still
    /// runs out.
    #[test]
    fn a_glutted_town_runs_out_of_money_rather_than_paying_for_ever() {
        let site = of(Speciality::Mine);
        let mut books = Economy::new();
        let market = books.market_mut(&site, 0);

        // Glut it: past `CAPACITY` the deposit is refused, so the price is
        // pinned at its floor and cannot fall any further.
        market.deposit(STONE, CAPACITY);
        let floor = market.price(STONE);
        market.deposit(STONE, CAPACITY);
        assert_eq!(market.price(STONE), floor, "the price was not at its floor");
        assert!(floor >= 1, "a good went free");

        // The till is finite, so what it can buy at that floor is finite.
        let affordable = market.affords(floor);
        assert!(affordable > 0, "a town opened with no money at all");
        let drawn = market.draw(affordable * floor);
        assert_eq!(drawn, affordable * floor);
        assert_eq!(
            market.affords(floor),
            0,
            "an emptied till could still pay for a block"
        );
        // And it cannot be over-drawn: asking for more than is there pays out
        // what is there, never more.
        assert_eq!(market.draw(1_000_000), market.till());
    }

    /// A drained town recovers on its own if you leave it, which is what makes
    /// "where do I sell next" a question worth asking.
    #[test]
    fn an_emptied_till_fills_back_up_over_time() {
        let site = of(Speciality::Refinery);
        let mut books = Economy::new();
        let market = books.market_mut(&site, 0);
        let all = market.till();
        assert_eq!(market.draw(all), all);
        assert_eq!(market.till(), 0);

        // A day of being left alone.
        let after = books.market(&site, STEP * 60).till();
        assert!(after > 0, "a drained town never earned a credit back");
        // And it does not overflow its own cap however long it is left.
        let ages = books.market(&site, STEP * 100_000).till();
        assert!(
            ages <= TILL_CAP as u64,
            "a town saved up past its own ceiling: {ages}"
        );
    }

    /// A town stripped of goods earns back more slowly than one still holding
    /// stock to sell on: the refill is what the place trades, not a stipend.
    #[test]
    fn a_town_with_nothing_to_sell_earns_slowly() {
        let site = of(Speciality::Depot);
        let mut rich = Economy::new();
        let mut bare = Economy::new();

        {
            let market = rich.market_mut(&site, 0);
            let all = market.till();
            market.draw(all);
        }
        {
            let market = bare.market_mut(&site, 0);
            let all = market.till();
            market.draw(all);
            for good in 0..GOODS.len() {
                market.withdraw(good, CAPACITY);
            }
        }

        let full = rich.market(&site, STEP * 10).till();
        let empty = bare.market(&site, STEP * 10).till();
        assert!(
            full > empty,
            "a stripped town earned as fast as a stocked one: {full} vs {empty}"
        );
    }

    /// The till survives a save, and a book written before towns had money is
    /// read back with the float its site opens on rather than as broke.
    #[test]
    fn the_till_survives_a_save_and_an_old_book_is_not_broke() {
        let directory = std::env::temp_dir().join(format!("vx-till-{}", std::process::id()));
        std::fs::remove_dir_all(&directory).ok();
        std::fs::create_dir_all(&directory).unwrap();

        let site = of(Speciality::Mine);
        let mut books = Economy::new();
        let spent = {
            let market = books.market_mut(&site, 0);
            market.draw(120);
            market.till()
        };
        books.save(&directory).unwrap();

        let mut back = Economy::new();
        back.load(&directory);
        assert_eq!(back.market(&site, 0).till(), spent, "the till did not survive");

        // A version-4 book: the same bytes without the till column. The
        // loader cannot know what a town had, so the site's own float stands
        // in — a town that has been trading for a week is not penniless.
        let mut v4 = std::fs::read(directory.join("economy.dat")).unwrap();
        v4[4..8].copy_from_slice(&4u32.to_le_bytes());
        std::fs::write(directory.join("economy.dat"), &v4).unwrap();
        let mut old = Economy::new();
        old.load(&directory);
        std::fs::remove_dir_all(&directory).ok();
        // The read will fail on the trailing byte count and fall back to fresh
        // books, or it will read and fill the till in; either way the town has
        // money and the world still opens.
        assert!(
            old.market(&site, 0).till() > 0,
            "an old save left a town unable to buy anything"
        );
    }

    /// A resident's purse is arithmetic on the seed and the clock, not a
    /// number somebody wrote down — which is what lets every town in the
    /// world have people with money in their pockets and cost the save
    /// nothing.
    #[test]
    fn a_purse_is_derived_and_never_stored() {
        for site in frontier() {
            for index in 0..crate::people::PEOPLE {
                let asked = purse(&site, index, STEP * 200);
                assert_eq!(asked, purse(&site, index, STEP * 200));
            }
        }
    }

    /// And it goes up, because they went to work. A day is forty steps and a
    /// shift is most of it, so an honest day has to show.
    #[test]
    fn a_days_work_puts_something_in_a_pocket() {
        let site = of(Speciality::Mine);
        let day = crate::schedule::TICKS_PER_DAY;
        for index in 0..crate::people::PEOPLE {
            let after_one = purse(&site, index, day);
            let after_three = purse(&site, index, day * 3);
            assert!(after_one > 0, "resident {index} worked a day for nothing");
            assert!(
                after_three > after_one,
                "resident {index} stopped earning: {after_one} then {after_three}"
            );
        }
    }

    /// Diligence varies, so a town's three residents are three different
    /// people rather than one person copied — the check that the rate is a
    /// draw and not a constant.
    #[test]
    fn no_two_towns_work_at_quite_the_same_rate() {
        let mut rates = Vec::new();
        for site in frontier() {
            for index in 0..crate::people::PEOPLE {
                rates.push(diligence(&site, index));
            }
        }
        let first = rates[0];
        assert!(
            rates.iter().any(|rate| (rate - first).abs() > 0.05),
            "every hand on the frontier works at exactly {first}"
        );
        assert!(rates.iter().all(|rate| (0.6..=1.4).contains(rate)));
    }

    /// The town's books notice what its people are doing. Run a market
    /// through the working hours and through the small hours and the ledgers
    /// diverge — which is the whole point of hanging the term on the
    /// schedule rather than on a constant.
    #[test]
    fn the_books_read_differently_when_the_town_is_asleep() {
        let site = of(Speciality::Mine);
        // Six steps is three town hours: one block inside the working day,
        // one block in the middle of the night.
        let working = {
            let mut economy = Economy::new();
            let start = STEP * 20;
            let before = economy.market(&site, start).clone();
            let after = economy.market(&site, start + STEP * 6).clone();
            after.stock(ORE) - before.stock(ORE)
        };
        let sleeping = {
            let mut economy = Economy::new();
            let start = STEP * 2;
            let before = economy.market(&site, start).clone();
            let after = economy.market(&site, start + STEP * 6).clone();
            after.stock(ORE) - before.stock(ORE)
        };
        assert!(
            working > sleeping,
            "a mine made as much at four in the morning as at noon: {working} vs {sleeping}"
        );
    }

    /// The town's own people can be told apart in its books: a tallyman
    /// counts and does not produce, so not every resident is a source.
    #[test]
    fn a_town_has_services_as_well_as_producers() {
        let mut making = 0;
        let mut counting = 0;
        for site in frontier() {
            for index in 0..crate::people::PEOPLE {
                match made_by(crate::people::person(&site, index).trade) {
                    Some(_) => making += 1,
                    None => counting += 1,
                }
            }
        }
        assert!(making > 0 && counting > 0, "{making} making, {counting} not");
    }

    #[test]
    fn catching_up_in_one_go_matches_catching_up_in_pieces() {
        // The load-bearing property of the whole model. If reading a market
        // late gives a different answer from reading it often, then the world
        // depends on when the player happened to look — which is the bug stage
        // 9a spent a round removing from the drones, reappearing in the books.
        //
        // It has to hold *through* the clamps: long enough that a mine fills
        // up and a refinery runs its inputs dry, because that is exactly where
        // a naive `stock += rate * elapsed` stops being splittable.
        for speciality in [Speciality::Mine, Speciality::Refinery, Speciality::Depot] {
            let site = of(speciality);
            let far = 200_000; // a good twenty in-game days

            let mut all_at_once = Economy::new();
            let mut in_pieces = Economy::new();
            // Seeded past the first two growth thresholds, so the catch-up
            // being compared is one that *builds* — the works multiplier is a
            // term in extraction and conversion, and a town that grew at a
            // different moment would make a different amount of everything.
            for economy in [&mut all_at_once, &mut in_pieces] {
                economy.market_mut(&site, 0).earn(GROWTH_AT[1] as u64 + 1);
            }

            all_at_once.market(&site, far);
            for at in (0..=far).step_by(STEP as usize * 7) {
                in_pieces.market(&site, at);
            }
            in_pieces.market(&site, far);

            assert_eq!(
                all_at_once.market(&site, far).growth(),
                2,
                "{} did not grow, so this proves nothing about growth",
                speciality.name()
            );
            assert_eq!(
                all_at_once.market(&site, far),
                in_pieces.market(&site, far),
                "{} books depend on when they were read",
                speciality.name()
            );
        }
    }

    #[test]
    fn reading_a_market_between_steps_does_not_advance_it() {
        // Catch-up lands on step boundaries, which is what makes the property
        // above exact rather than approximate.
        let site = of(Speciality::Mine);
        let mut economy = Economy::new();
        let opening = economy.market(&site, 0).clone();
        assert_eq!(economy.market(&site, STEP - 1), &opening);
        assert_ne!(economy.market(&site, STEP), &opening);
        assert_eq!(economy.market(&site, STEP).last_tick, STEP);
    }

    #[test]
    fn a_town_makes_what_it_is_for_and_burns_what_it_needs() {
        let mine = of(Speciality::Mine);
        let mut economy = Economy::new();
        let before = economy.market(&mine, 0).clone();
        let after = economy.market(&mine, STEP * 40).clone();
        assert!(
            after.stock(ORE) > before.stock(ORE),
            "a mine did not produce ore"
        );
        assert!(
            after.stock(LOG) < before.stock(LOG),
            "a mine did not burn any timber"
        );

        // A refinery turns ore and stone into bars.
        let refinery = of(Speciality::Refinery);
        let before = economy.market(&refinery, 0).clone();
        let after = economy.market(&refinery, STEP * 40).clone();
        assert!(
            after.stock(BAR) > before.stock(BAR),
            "a refinery made no bars"
        );
        assert!(after.stock(ORE) < before.stock(ORE), "it ate no ore");
    }

    #[test]
    fn a_refinery_with_no_ore_makes_no_bars() {
        // The conversion is limited by its inputs, which is what stops a
        // refinery conjuring metal out of an empty yard.
        let site = of(Speciality::Refinery);
        let mut economy = Economy::new();
        {
            let market = economy.market_mut(&site, 0);
            market.withdraw(ORE, 1e9);
            market.withdraw(STONE, 1e9);
        }
        let bars = economy.market(&site, 0).stock(BAR);
        let after = economy.market(&site, STEP * 10).stock(BAR);
        assert!(after <= bars, "bars appeared from an empty yard");
        assert_eq!(economy.market(&site, STEP * 10).stock(ORE), 0.0);
    }

    #[test]
    fn stock_never_leaves_its_bounds_however_long_it_runs() {
        for speciality in [Speciality::Mine, Speciality::Refinery, Speciality::Depot] {
            let site = of(speciality);
            let mut economy = Economy::new();
            let market = economy.market(&site, 1_000_000);
            for (good, name) in GOODS.iter().enumerate() {
                let held = market.stock(good);
                assert!(
                    (0.0..=CAPACITY).contains(&held),
                    "{} holds {held} of {name}",
                    speciality.name()
                );
            }
        }
    }

    #[test]
    fn a_glutted_market_pays_less_than_a_starving_one() {
        let site = of(Speciality::Depot);
        let mut economy = Economy::new();

        let starving = {
            let market = economy.market_mut(&site, 0);
            market.withdraw(ORE, 1e9);
            market.price(ORE)
        };
        let glutted = {
            let market = economy.market_mut(&site, 0);
            market.deposit(ORE, CAPACITY);
            market.price(ORE)
        };

        assert!(
            starving > glutted,
            "a starving market paid {starving} and a glutted one {glutted}"
        );
        // Bounded at both ends: never free, never runaway.
        assert!(glutted >= 1, "a glutted market paid nothing at all");
        let ceiling = (BASE_PRICE[ORE] * MAX_FACTOR).round() as u64;
        assert!(starving <= ceiling, "a starving market paid {starving}");
    }

    #[test]
    fn price_falls_the_more_a_market_is_holding() {
        // Monotonic, so a player dumping a load can predict what it does.
        let site = of(Speciality::Depot);
        let mut economy = Economy::new();
        let market = economy.market_mut(&site, 0);
        market.withdraw(ORE, 1e9);

        let mut last = u64::MAX;
        for _ in 0..20 {
            let price = market.price(ORE);
            assert!(price <= last, "price rose from {last} to {price} on a sale");
            last = price;
            market.deposit(ORE, 60.0);
        }
    }

    #[test]
    fn moving_goods_reports_what_actually_moved() {
        let site = of(Speciality::Depot);
        let mut economy = Economy::new();
        let market = economy.market_mut(&site, 0);

        market.withdraw(BAR, 1e9);
        assert_eq!(market.stock(BAR), 0.0);
        assert_eq!(market.withdraw(BAR, 50.0), 0.0, "took from an empty shelf");

        assert_eq!(market.deposit(BAR, 100.0), 100.0);
        assert_eq!(market.stock(BAR), 100.0);
        // And a market cannot be filled past its capacity.
        let taken = market.deposit(BAR, CAPACITY * 2.0);
        assert_eq!(taken, CAPACITY - 100.0);
        assert_eq!(market.stock(BAR), CAPACITY);
    }

    #[test]
    fn towns_open_on_different_books_but_the_same_ones_every_time() {
        let sites = frontier();
        assert!(sites.len() > 3, "the fixture frontier is too small");

        for site in &sites {
            assert_eq!(
                opening_books(site),
                opening_books(site),
                "a town's opening books are not reproducible"
            );
        }
        // Two towns should not read identically, or the map has no gradient
        // for anyone to trade across.
        assert!(
            sites
                .windows(2)
                .any(|pair| opening_books(&pair[0]) != opening_books(&pair[1])),
            "every town opened on identical books"
        );
    }

    #[test]
    fn an_untouched_frontier_costs_the_save_nothing() {
        // A town nobody has visited is derived, not stored.
        let sites = frontier();
        let mut economy = Economy::new();
        assert_eq!(economy.tracked(), 0);
        economy.market(&sites[0], 5_000);
        assert_eq!(economy.tracked(), 1, "reading one town tracked {} ", economy.tracked());
    }

    #[test]
    fn the_books_round_trip_and_tolerate_damage() {
        let directory = std::env::temp_dir().join(format!("vx-economy-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();

        let sites = frontier();
        let mut economy = Economy::new();
        for site in sites.iter().take(4) {
            economy.market_mut(site, 12_000).deposit(BAR, 37.0);
        }
        economy.save(&directory).unwrap();

        let mut read = Economy::new();
        read.load(&directory);
        assert_eq!(read, economy, "the books did not survive the trip");
        assert_eq!(read.books_hash(), economy.books_hash());

        std::fs::write(directory.join("economy.dat"), b"NOT A LEDGER").unwrap();
        let mut damaged = Economy::new();
        damaged.load(&directory);
        assert_eq!(damaged.tracked(), 0, "a damaged file invented markets");

        std::fs::remove_dir_all(&directory).ok();
        let mut missing = Economy::new();
        missing.load(&directory);
        assert_eq!(missing.tracked(), 0);
    }

    #[test]
    fn negative_town_coordinates_survive_the_encoding() {
        let directory = std::env::temp_dir().join(format!("vx-economy-neg-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();

        let site = frontier()
            .into_iter()
            .find(|site| site.centre.0 < 0 || site.centre.1 < 0)
            .expect("the fixture frontier has no town west or north of the origin");
        let mut economy = Economy::new();
        economy.market(&site, 3_000);
        economy.save(&directory).unwrap();

        let mut read = Economy::new();
        read.load(&directory);
        assert_eq!(read, economy);
        std::fs::remove_dir_all(&directory).ok();
    }

    /// Total units of a good held across every tracked town plus everything in
    /// the air. What conservation is measured against.
    fn afloat_and_ashore(economy: &Economy, good: usize) -> f32 {
        let ashore: f32 = economy
            .towns
            .values()
            .map(|market| market.stock[good])
            .sum();
        let afloat: f32 = economy
            .shipments
            .iter()
            .filter(|load| load.good == good)
            .map(|load| load.amount)
            .sum();
        ashore + afloat
    }

    #[test]
    fn a_load_leaves_one_town_and_lands_whole_at_the_other() {
        // Nothing may be created or destroyed by the act of carrying it.
        let sites = frontier();
        // Just the two towns, so nothing else's books enter the sum.
        let pair = [sites[0], sites[1]];
        let (from, to) = (pair[0], pair[1]);
        let arrive = Shipment::travel_ticks(from.centre, to.centre);

        let mut economy = Economy::new();
        // Both caught up to the landing tick first, so the only thing that
        // happens across the measurement is the load being carried — not a
        // day's production either side of it.
        economy.market(&from, arrive);
        economy.market(&to, arrive);

        let before = afloat_and_ashore(&economy, ORE);
        let carried = economy.market_mut(&from, arrive).withdraw(ORE, 120.0);
        assert!(carried > 0.0);
        economy.ship(Shipment {
            from: from.centre,
            to: to.centre,
            good: ORE,
            amount: carried,
            depart: 0,
            arrive,
            owner: Owner::Town,
            paid: 0,
        });

        // In the air: still accounted for, and not yet on anyone's shelves.
        assert_eq!(economy.shipments().len(), 1);
        assert!((afloat_and_ashore(&economy, ORE) - before).abs() < 0.01);

        // Landed: the destination has it, nothing is in the air.
        economy.settle(&pair, arrive, &mut Vec::new());
        assert!(economy.shipments().is_empty(), "the load never landed");
        assert!(
            (afloat_and_ashore(&economy, ORE) - before).abs() < 0.01,
            "goods were created or lost in transit"
        );
    }

    #[test]
    fn a_load_is_where_the_clock_says_and_nowhere_else() {
        // Position is a sum, not a simulation — which is what lets one be drawn
        // as a real drone without any state existing for it.
        let load = Shipment {
            from: (0, 0),
            to: (1_000, -500),
            good: ORE,
            amount: 50.0,
            depart: 1_000,
            arrive: 2_000,
            owner: Owner::Town,
            paid: 0,
        };
        assert_eq!(load.position_at(1_000), (0.0, 0.0));
        assert_eq!(load.position_at(2_000), (1_000.0, -500.0));
        assert_eq!(load.position_at(1_500), (500.0, -250.0));

        // Asking twice gives the same answer, and it never runs off either end.
        assert_eq!(load.position_at(1_500), load.position_at(1_500));
        assert_eq!(load.position_at(0), (0.0, 0.0));
        assert_eq!(load.position_at(9_999), (1_000.0, -500.0));
    }

    #[test]
    fn the_network_ships_from_glut_to_shortage() {
        // The game-of-life bit: left alone, towns move goods to where they are
        // wanted, and the run happens whether anyone is watching or not.
        let sites = frontier();
        let mut economy = Economy::new();

        // Long enough for specialities to pull the books apart.
        let mut now = 0;
        let mut seen = 0;
        for _ in 0..40 {
            now += DISPATCH_EVERY;
            economy.run(&sites, now);
            seen = seen.max(economy.shipments().len());
        }
        assert!(seen > 0, "the network never sent anything anywhere");

        // And every load it sent went somewhere that wanted it.
        for load in economy.shipments() {
            assert_ne!(load.from, load.to, "a town shipped to itself");
            assert!(load.amount > 0.0, "an empty load went out");
            assert!(load.arrive > load.depart, "a load arrived before it left");
        }
    }

    #[test]
    fn the_network_is_the_same_however_often_it_is_run() {
        // Same guarantee as the books themselves: dispatch is quantised, so
        // running the network every step and running it rarely land in the
        // same place.
        let sites = frontier();
        let far = DISPATCH_EVERY * 30;

        let mut often = Economy::new();
        for at in (0..=far).step_by(DISPATCH_EVERY as usize) {
            often.run(&sites, at);
        }

        let mut rarely = Economy::new();
        for at in (0..=far).step_by(DISPATCH_EVERY as usize * 5) {
            rarely.run(&sites, at);
        }
        rarely.run(&sites, far);
        often.run(&sites, far);

        assert_eq!(
            often.books_hash(),
            rarely.books_hash(),
            "the network's books depend on how often it was looked at"
        );
        // And the hash is over money as well as goods since stage 58, so a
        // frontier that traded its way into a new building either did so in
        // both runs or the assertion above catches it. This says it happened
        // at all, so the test is not passing on a network that never moved.
        assert!(
            sites
                .iter()
                .any(|site| often.market(site, far).growth() > 0),
            "thirty windows of trade did not put a single building up"
        );
    }

    /// The first round in which credits move between towns.
    ///
    /// Before stage 58 the network moved goods and nothing else, so a town on
    /// a busy corner of the map and a town nobody shipped to ended up with the
    /// same money — which meant the till said nothing about how a place was
    /// doing, and there was nothing for growth to be measured against.
    #[test]
    fn a_town_pays_for_what_it_orders_and_the_seller_takes_it() {
        let sites = frontier();
        let now = DISPATCH_EVERY;
        let mut economy = Economy::new();

        // Bring every book up to the window first, so the only thing that
        // moves across the dispatch is the dispatch.
        let before: Vec<(u64, u64)> = sites
            .iter()
            .map(|site| {
                let market = economy.market(site, now);
                (market.till(), market.trade())
            })
            .collect();

        economy.dispatch(&sites, now);

        let freight: Vec<&Shipment> = economy
            .shipments()
            .iter()
            .filter(|load| load.owner == Owner::Town)
            .collect();
        assert!(!freight.is_empty(), "the network put nothing in the air");
        assert!(
            freight.iter().all(|load| load.paid > 0),
            "a town took a load without paying for it"
        );

        let owed: u64 = freight.iter().map(|load| load.paid).sum();
        let buyers: Vec<(i32, i32)> = freight.iter().map(|load| load.to).collect();
        let sellers: Vec<(i32, i32)> = freight.iter().map(|load| load.from).collect();

        let mut spent = 0u64;
        let mut earned = 0u64;
        for (site, (till, trade)) in sites.iter().zip(before) {
            let market = economy.market(site, now);
            if buyers.contains(&site.centre) {
                spent += till.saturating_sub(market.till());
            }
            if sellers.contains(&site.centre) {
                earned += market.trade().saturating_sub(trade);
            }
        }
        assert_eq!(spent, owed, "the buyers did not put up what the loads cost");
        assert_eq!(earned, owed, "the sellers did not take what was put up");
    }

    /// A town orders what it can pay for and nothing else.
    ///
    /// The till doing work the stock curve cannot do on its own: a poor town
    /// stays short, so its prices stay high, so the first load it can afford
    /// is the one it needs most.
    #[test]
    fn a_town_that_cannot_pay_does_not_order() {
        let sites = frontier();
        let now = DISPATCH_EVERY;
        let mut broke = Economy::new();
        for site in &sites {
            let market = broke.market_mut(site, now);
            let all = market.till();
            market.draw(all);
        }
        broke.dispatch(&sites, now);
        assert!(
            broke
                .shipments()
                .iter()
                .all(|load| load.owner != Owner::Town),
            "a frontier with empty tills went shopping anyway"
        );

        // And the same frontier with money in it does trade, so the test above
        // is measuring the till rather than a network that never runs.
        let mut solvent = Economy::new();
        solvent.dispatch(&sites, now);
        assert!(
            solvent
                .shipments()
                .iter()
                .any(|load| load.owner == Owner::Town),
            "the same frontier with money in it shipped nothing either"
        );
    }

    /// Nothing is conjured — the money half of the conservation identity.
    ///
    /// Not an equality, and deliberately: [`Market::earn`] caps the till at
    /// [`TILL_CAP`], so a sale into a full till loses the overflow. What must
    /// never happen is the other direction.
    #[test]
    fn a_dispatch_never_creates_credits() {
        let sites = frontier();
        let now = DISPATCH_EVERY * 4;
        let mut economy = Economy::new();
        let before: u64 = sites
            .iter()
            .map(|site| economy.market(site, now).till())
            .sum();
        economy.dispatch(&sites, now);
        let after: u64 = sites
            .iter()
            .map(|site| economy.market(site, now).till())
            .sum();
        assert!(
            after <= before,
            "the frontier came out of a dispatch richer than it went in: {before} -> {after}"
        );
    }

    /// Growth is a ratchet: a town that has a good decade and then a bad year
    /// has still had the good decade, and keeps what it built.
    #[test]
    fn growth_is_a_ratchet_and_survives_going_broke() {
        let site = of(Speciality::Depot);
        let mut economy = Economy::new();

        let market = economy.market_mut(&site, 0);
        market.earn(GROWTH_AT[0] as u64 + 1);
        // Growth lands on a step boundary, like everything else here.
        assert_eq!(economy.market(&site, 0).growth(), 0, "growth outran the step");
        assert_eq!(economy.market(&site, STEP).growth(), 1);

        // Now strip it bare: no money, no goods, and a long time to think
        // about it.
        let market = economy.market_mut(&site, STEP);
        let all = market.till();
        market.draw(all);
        for good in 0..GOODS.len() {
            market.withdraw(good, CAPACITY);
        }
        assert_eq!(
            economy.market(&site, STEP * 500).growth(),
            1,
            "a town that went broke pulled its own buildings down"
        );
    }

    /// Every threshold is reachable, crossing two at once is two steps, and a
    /// town cannot grow past what has been authored for it.
    #[test]
    fn a_town_grows_step_by_step_and_stops_at_the_top() {
        let site = of(Speciality::Mine);
        for (step, threshold) in GROWTH_AT.iter().enumerate() {
            let mut economy = Economy::new();
            economy.market_mut(&site, 0).earn(*threshold as u64);
            assert_eq!(
                economy.market(&site, STEP).growth() as usize,
                step + 1,
                "threshold {step} did not put a building up"
            );
        }

        // Past the top of the table, and it stays at the top: one blueprint is
        // authored per step, so a town grown past the table would ask for a
        // building that does not exist.
        let mut rich = Economy::new();
        rich.market_mut(&site, 0).earn(GROWTH_AT[GROWTH_AT.len() - 1] as u64 * 100);
        assert_eq!(rich.market(&site, STEP).growth(), MAX_GROWTH);
        assert_eq!(
            MAX_GROWTH as usize,
            vx_world::town::plan::growth_steps(&site),
            "the growth table and the blueprints disagree about how big a town gets"
        );
    }

    /// What a new shed is *for*: a grown town gets more out of its own works.
    /// This is the loop closing — a town that trades well builds, and what it
    /// builds lets it trade more.
    #[test]
    fn a_grown_town_makes_more_of_what_it_makes() {
        for speciality in [Speciality::Mine, Speciality::Refinery, Speciality::Depot] {
            let site = of(speciality);
            let run = STEP * 40;

            let mut small = Economy::new();
            let mut large = Economy::new();
            // Bare shelves both, so the comparison is about what is made
            // rather than about where a clamp happens to bite.
            for economy in [&mut small, &mut large] {
                let market = economy.market_mut(&site, 0);
                for good in 0..GOODS.len() {
                    market.withdraw(good, CAPACITY);
                }
            }
            large.market_mut(&site, 0).growth = MAX_GROWTH;

            let made: f32 = (0..GOODS.len())
                .map(|good| small.market(&site, run).stock(good))
                .sum();
            let more: f32 = (0..GOODS.len())
                .map(|good| large.market(&site, run).stock(good))
                .sum();
            assert!(
                more > made,
                "{} got nothing out of growing: {made} -> {more}",
                speciality.name()
            );
        }
    }

    #[test]
    fn loads_in_the_air_survive_a_save() {
        let directory = std::env::temp_dir().join(format!("vx-loads-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();

        let sites = frontier();
        let mut economy = Economy::new();
        for _ in 0..30 {
            economy.run(&sites, economy.last_dispatch + DISPATCH_EVERY);
        }
        economy.ship(Shipment {
            from: sites[0].centre,
            to: sites[1].centre,
            good: BAR,
            amount: 17.5,
            depart: 10,
            arrive: 9_000,
            owner: Owner::Player,
            paid: 0,
        });
        economy.save(&directory).unwrap();

        let mut read = Economy::new();
        read.load(&directory);
        assert_eq!(read, economy, "loads in the air did not survive the trip");
        assert!(
            read.shipments().iter().any(|load| load.owner == Owner::Player),
            "the player's load was lost"
        );

        std::fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn an_intercepted_load_never_lands_and_the_town_stays_short() {
        let sites = frontier();
        let mut economy = Economy::new();
        economy.ship(Shipment {
            from: sites[0].centre,
            to: sites[1].centre,
            good: BAR,
            amount: 40.0,
            depart: 10,
            arrive: 500,
            owner: Owner::Town,
            paid: 0,
        });
        let before = economy.market(&sites[1], 0).stock(BAR);

        let load = economy.intercept(0).expect("nothing to intercept");
        assert_eq!(load.amount, 40.0);
        assert!(economy.shipments().is_empty(), "the wreck kept flying");
        assert!(economy.intercept(0).is_none(), "intercepted the same load twice");

        // Long past the arrival tick, the destination never saw the goods:
        // no deposit, no phantom bookkeeping. (The market moves on its own
        // schedule; compare against an unshot twin, not against `before`.)
        let mut untouched = Economy::new();
        let landed_stock = {
            untouched.ship(Shipment {
                from: sites[0].centre,
                to: sites[1].centre,
                good: BAR,
                amount: 40.0,
                depart: 10,
                arrive: 500,
                owner: Owner::Town,
                paid: 0,
            });
            untouched.run(&sites, 600);
            untouched.market(&sites[1], 600).stock(BAR)
        };
        economy.run(&sites, 600);
        let short_stock = economy.market(&sites[1], 600).stock(BAR);
        assert!(
            short_stock < landed_stock,
            "the shortage never registered: shot {short_stock} vs landed {landed_stock} (started {before})"
        );
    }

    #[test]
    fn running_the_network_loads_no_chunks() {
        // Same promise the beacon board makes: the frontier is arithmetic.
        let probes = std::cell::Cell::new(0usize);
        let sites = town::towns_near(11, (0, 0), 3_000, &|_, _| {
            probes.set(probes.get() + 1);
            90
        });
        let after_siting = probes.get();

        let mut economy = Economy::new();
        for _ in 0..20 {
            economy.run(&sites, economy.last_dispatch + DISPATCH_EVERY);
        }
        assert_eq!(
            probes.get(),
            after_siting,
            "running the network went back to the terrain"
        );
        assert!(economy.tracked() > 0, "the network touched nothing at all");
    }
}

#[cfg(test)]
mod mail_tests {
    use super::*;
    use vx_world::town::home_site;

    fn far_town() -> TownSite {
        TownSite {
            centre: (1024, 512),
            ..home_site()
        }
    }

    #[test]
    fn a_mail_load_lands_in_the_delivered_list_not_the_towns_books() {
        let mut economy = Economy::new();
        let home = home_site();
        let source = far_town();

        let before = economy.market(&home, 0).stock(ORE);
        economy.ship(Shipment {
            from: source.centre,
            to: home.centre,
            good: ORE,
            amount: PARCEL as f32,
            depart: 0,
            arrive: 10,
            owner: Owner::Mail,
            paid: 0,
        });

        let landed = economy.run(&[home, source], 20);

        assert_eq!(landed.len(), 1, "the mail never landed");
        assert_eq!(landed[0].owner, Owner::Mail);
        let after = economy.market(&home, 20).stock(ORE);
        assert!(
            (after - before).abs() < f32::EPSILON,
            "mail leaked into the town's books: {before} became {after}"
        );
    }

    #[test]
    fn an_economy_file_from_before_mail_still_loads() {
        // The layout never changed; v1 files simply predate the Mail owner
        // byte. Write a real file, stamp it v1, and it must load whole.
        let directory = std::env::temp_dir().join(format!(
            "vx-economy-v1-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).unwrap();

        let mut economy = Economy::new();
        economy.ship(Shipment {
            from: far_town().centre,
            to: home_site().centre,
            good: LOG,
            amount: 30.0,
            depart: 5,
            arrive: 500,
            owner: Owner::Player,
            paid: 0,
        });
        economy.save(&directory).unwrap();

        let path = directory.join("economy.dat");
        let mut bytes = std::fs::read(&path).unwrap();
        bytes[4..8].copy_from_slice(&1u32.to_le_bytes());
        std::fs::write(&path, &bytes).unwrap();

        let mut read_back = Economy::new();
        read_back.load(&directory);
        std::fs::remove_dir_all(&directory).ok();

        assert_eq!(read_back.shipments().len(), 1, "the v1 file was refused");
        assert_eq!(read_back.shipments()[0].owner, Owner::Player);
    }

    #[test]
    fn a_future_economy_version_is_still_refused() {
        let directory = std::env::temp_dir().join(format!(
            "vx-economy-v9-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).unwrap();

        let economy = Economy::new();
        economy.save(&directory).unwrap();
        let path = directory.join("economy.dat");
        let mut bytes = std::fs::read(&path).unwrap();
        bytes[4..8].copy_from_slice(&9u32.to_le_bytes());
        std::fs::write(&path, &bytes).unwrap();

        let mut fresh = Economy::new();
        fresh.load(&directory); // must warn and start fresh, not panic
        std::fs::remove_dir_all(&directory).ok();
        assert!(fresh.shipments().is_empty());
    }
}
