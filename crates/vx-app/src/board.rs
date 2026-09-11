//! The beacon console: the panel the radio mast puts in front of you.
//!
//! The shop's twin (overlay slot 5), and deliberately so — the same cursor,
//! the same Enter-confirms, the same one-line feedback. What differs is where
//! the rows come from: the shop sells from a table, the board reads work off
//! [`crate::beacon::postings_for`], which is derived from the town rather than
//! stored anywhere.
//!
//! Settlement lives here rather than in [`crate::beacon`] because it is the
//! only place that has the pile, the wallet and the survey record to hand at
//! once; the ledger itself stays a record of what happened, not a rule about
//! what may.

use vx_agent::Stockpile;
use vx_render::font::{self, LINE_HEIGHT};
use vx_world::town::TownSite;

use vx_world::World;

use crate::beacon::{Ledger, Posting, Task};
use crate::economy::{self, Market};
use crate::map::{self, MapState, Marker};
use crate::shop::display_name;
use crate::wallet::Wallet;

/// Panel size in texture pixels; displayed at [`BOARD_SCALE`].
pub const BOARD_WIDTH: u32 = 256;
pub const BOARD_HEIGHT: u32 = 262;

/// Side of the inset trade map, in panel pixels.
pub const TRADE_MAP: u32 = 96;
pub const BOARD_SCALE: f32 = 2.0;

const TEXT: [u8; 4] = [235, 235, 235, 255];
const DIM: [u8; 4] = [150, 150, 155, 255];
const ACCENT: [u8; 4] = [255, 170, 60, 255];
const GOOD: [u8; 4] = [120, 220, 120, 255];
/// What the town has out on you, and anything else it will not do for you.
const WARN: [u8; 4] = [225, 95, 85, 255];
const BACKGROUND: [u8; 4] = [10, 12, 16, 235];

/// One selectable line of the board.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    /// Work due here, ready to sign for.
    HandIn(u64),
    /// Work on offer here.
    Accept(u64),
    /// On offer here, but already on your sheet.
    Taken(u64),
    /// Put a load of your own on the network: this good, to that town.
    Ship { good: usize, to: (i32, i32) },
}

impl Row {
    /// The posting this row is about, if it is about one at all.
    pub fn id(&self) -> Option<u64> {
        match self {
            Row::HandIn(id) | Row::Accept(id) | Row::Taken(id) => Some(*id),
            Row::Ship { .. } => None,
        }
    }
}

/// Which of the console's pages is up.
///
/// The handheld established the idiom in stage 33: one panel, several pages,
/// turned with a key. The board grew a civic block in 39 and would have grown
/// a whole election into the same column here, which is a screen nobody can
/// read — so the ballot gets a page of its own.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Page {
    /// Prices, who runs the place, and the work on offer.
    #[default]
    Work,
    /// The ballot box.
    Ballot,
}

impl Page {
    pub fn turned(self) -> Self {
        match self {
            Page::Work => Page::Ballot,
            Page::Ballot => Page::Work,
        }
    }
}

/// The console's interaction state.
#[derive(Debug, Default)]
pub struct Board {
    pub open: bool,
    pub page: Page,
    cursor: usize,
    /// The last action's outcome, shown until the next one.
    pub feedback: Option<String>,
}

impl Board {
    pub fn new() -> Self {
        Board::default()
    }

    pub fn open_at_beacon(&mut self) {
        self.open = true;
        self.page = Page::Work;
        self.cursor = 0;
        self.feedback = None;
    }

    /// Turn to the other page. The cursor starts again, because the rows on
    /// the two pages are about entirely different things.
    pub fn turn_page(&mut self) {
        self.page = self.page.turned();
        self.cursor = 0;
        self.feedback = None;
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn close(&mut self) {
        self.open = false;
        self.feedback = None;
    }

    /// The rows this console shows: work due here first, because arriving
    /// with a full pack and having to scroll past adverts would be rude.
    pub fn rows(here: (i32, i32), postings: &[Posting], ledger: &Ledger) -> Vec<Row> {
        Board::rows_with_runs(here, postings, ledger, &[])
    }

    /// The board's rows, plus any trade runs this town is offering.
    ///
    /// `runs` is what the network will carry from here and where to — computed
    /// by the caller, which is the only place that knows the neighbours' books.
    pub fn rows_with_runs(
        here: (i32, i32),
        postings: &[Posting],
        ledger: &Ledger,
        runs: &[Run],
    ) -> Vec<Row> {
        let mut rows = Board::postings_rows(here, postings, ledger);
        for run in runs {
            rows.push(Row::Ship {
                good: run.good,
                to: run.to,
            });
        }
        rows
    }

    fn postings_rows(here: (i32, i32), postings: &[Posting], ledger: &Ledger) -> Vec<Row> {
        let mut rows: Vec<Row> = ledger
            .due_at(here)
            .iter()
            .map(|posting| Row::HandIn(posting.id))
            .collect();
        for posting in postings {
            if ledger.is_settled(posting.id) {
                continue;
            }
            if ledger.is_accepted(posting.id) {
                // A posting settled at its issuer is already listed above.
                if !rows.iter().any(|row| row.id() == Some(posting.id)) {
                    rows.push(Row::Taken(posting.id));
                }
            } else {
                rows.push(Row::Accept(posting.id));
            }
        }
        rows
    }

    /// The row the cursor is on.
    pub fn selected(&self, rows: &[Row]) -> Option<Row> {
        rows.get(self.cursor).cloned()
    }

    /// Put a load of the player's own on the network.
    ///
    /// Takes the goods out of the base pile — exactly as the shop does — and
    /// hands them to [`crate::economy::Economy::ship`]. Nothing is paid here:
    /// a run pays at the far end, on arrival, which is what makes it a journey
    /// rather than a transaction.
    pub fn ship(
        &mut self,
        here: &TownSite,
        good: usize,
        to: (i32, i32),
        pile: Option<&mut Stockpile>,
        economy: &mut crate::economy::Economy,
        now: u64,
    ) {
        let name = economy::GOODS[good];
        let Some(pile) = pile else {
            self.feedback = Some("NO BASE PILE".into());
            return;
        };
        let held = pile.count(name);
        if held < economy::PLAYER_LOAD {
            self.feedback = Some(format!(
                "NEED {} {}, HAVE {held}",
                economy::PLAYER_LOAD,
                crate::shop::display_name(name)
            ));
            return;
        }

        let taken = pile.take(name, economy::PLAYER_LOAD);
        economy.ship(crate::economy::Shipment {
            from: here.centre,
            to,
            good,
            amount: taken as f32,
            depart: now,
            arrive: now + crate::economy::Shipment::travel_ticks(here.centre, to),
            owner: crate::economy::Owner::Player,
            // Paid on arrival, at the far town's price, not now.
            paid: 0,
        });
        self.feedback = Some(format!(
            "{} AWAY TO {} {}",
            crate::shop::display_name(name),
            to.0,
            to.1
        ));
    }

    pub fn move_cursor(&mut self, delta: i32, rows: usize) {
        if rows == 0 {
            self.cursor = 0;
            return;
        }
        self.cursor = (self.cursor as i32 + delta).clamp(0, rows as i32 - 1) as usize;
    }

    /// Act on the selected row.
    ///
    /// `surveyed` answers "has the fleet swept the sector holding this
    /// column?" — passed in rather than read, so this stays testable without
    /// a fleet and the module keeps knowing nothing about fliers.
    pub fn confirm(
        &mut self,
        here: &TownSite,
        postings: &[Posting],
        ledger: &mut Ledger,
        pile: Option<&mut Stockpile>,
        walletbook: &mut Wallet,
        surveyed: &dyn Fn((i32, i32)) -> bool,
    ) {
        let rows = Board::rows(here.centre, postings, ledger);
        let Some(row) = rows.get(self.cursor).cloned() else {
            self.feedback = Some("NO WORK POSTED".into());
            return;
        };

        match row {
            // Handled by `ship`, which needs the network and the pile rather
            // than the ledger. The caller checks for it before calling here.
            Row::Ship { .. } => {
                self.feedback = Some("PICK A LOAD AND PRESS ENTER".into());
            }
            Row::Taken(_) => {
                self.feedback = Some("ALREADY ON YOUR SHEET".into());
            }
            Row::Accept(id) => {
                let Some(posting) = postings.iter().find(|posting| posting.id == id) else {
                    self.feedback = Some("POSTING WITHDRAWN".into());
                    return;
                };
                if ledger.accept(posting) {
                    self.feedback = Some(match &posting.task {
                        Task::Deliver { target, .. } => {
                            format!("ACCEPTED. PINNED {}", target.name)
                        }
                        Task::Survey { .. } => "ACCEPTED. SEND THE FLIER".into(),
                    });
                } else {
                    self.feedback = Some("ALREADY ON YOUR SHEET".into());
                }
            }
            Row::HandIn(id) => {
                self.feedback = Some(settle(id, ledger, pile, walletbook, surveyed));
            }
        }

        let rows = Board::rows(here.centre, postings, ledger).len().max(1);
        self.cursor = self.cursor.min(rows - 1);
    }
}

/// Try to sign off one accepted posting. Reports what happened either way.
///
/// Atomic in the sense that matters: a contract that cannot be paid takes
/// nothing from the pile.
fn settle(
    id: u64,
    ledger: &mut Ledger,
    pile: Option<&mut Stockpile>,
    walletbook: &mut Wallet,
    surveyed: &dyn Fn((i32, i32)) -> bool,
) -> String {
    let Some(posting) = ledger.get(id).cloned() else {
        return "NOT ON YOUR SHEET".into();
    };

    match &posting.task {
        Task::Deliver {
            goods,
            amount,
            target,
        } => {
            let Some(pile) = pile else {
                return "NO BASE PILE".into();
            };
            let held = pile.count(goods);
            if held < *amount {
                return format!(
                    "NEED {amount} {}, HAVE {held}",
                    display_name(goods)
                );
            }
            // `take` reports what actually left, so the books balance even if
            // the pile changed under us.
            let taken = pile.take(goods, *amount);
            walletbook.earn(posting.reward);
            ledger.settle(id);
            format!(
                "SIGNED FOR {taken} {} AT {}. +{} CR",
                display_name(goods),
                target.name,
                posting.reward
            )
        }
        Task::Survey { at } => {
            if !surveyed(*at) {
                return "SECTOR NOT SWEPT YET".into();
            }
            walletbook.earn(posting.reward);
            ledger.settle(id);
            format!("SURVEY FILED. +{} CR", posting.reward)
        }
    }
}

/// Render the panel. Pure in its inputs.
/// A run this town will pay somebody to carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub good: usize,
    pub to: (i32, i32),
    /// The destination's name, carried so the panel need not go back to the
    /// lattice every frame to print it.
    pub name: String,
}

/// Where a trade run is going, and what the network has in the air.
pub struct TradeView<'a> {
    pub world: &'a World,
    pub explored: &'a MapState,
    /// Live caravans, as columns.
    pub traffic: &'a [(i32, i32)],
}

/// Who runs the town, and what the town has out on you.
///
/// Carried as strings rather than as the offices and the docket themselves,
/// for the reason every panel in this project is: the panel is a pure
/// function over a snapshot, and a renderer that could reach into the
/// warrant ledger would be a renderer that could change it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Civic {
    /// `("MAYOR", "GRANT THE MEEK")`, in office order.
    pub seats: Vec<(String, String)>,
    /// How the player came to hold this town, when not by the ballot box:
    /// `FOUNDED BY YOU - DAY 12`, `TAKEN - DAY 30`.
    pub charter: Option<String>,
    /// The warrant line, when there is one.
    pub warrant: Option<String>,
    /// Whether the town will trade with you at all.
    pub closed: bool,
}

/// What the ballot page draws, as a snapshot.
///
/// Strings and flags rather than the register itself, for the reason stage
/// 39's [`Civic`] is: a renderer that could reach into the election ledger is
/// a renderer that could change it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ballot {
    /// One row per seat: the title, who holds it, whether it is yours, and
    /// whether your name is on the ballot for it.
    pub seats: Vec<BallotSeat>,
    /// How many days until this town next votes.
    pub days_to_poll: u32,
    /// Each resident's leaning, already worded.
    pub leanings: Vec<String>,
}

/// One seat on the ballot page.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BallotSeat {
    pub title: String,
    pub holder: String,
    pub yours: bool,
    pub standing: bool,
}

/// Everything the town itself contributes to the panel.
pub struct Counter<'a> {
    pub here: &'a TownSite,
    pub postings: &'a [Posting],
    pub runs: &'a [Run],
    pub market: &'a Market,
    pub civic: &'a Civic,
    pub ballot: &'a Ballot,
}

pub fn render_board(
    board: &Board,
    counter: &Counter<'_>,
    ledger: &Ledger,
    walletbook: &Wallet,
    view: Option<&TradeView<'_>>,
) -> Vec<u8> {
    let Counter {
        here,
        postings,
        runs,
        market,
        civic,
        ballot,
    } = *counter;
    let mut pixels = vec![0u8; (BOARD_WIDTH * BOARD_HEIGHT * 4) as usize];
    for texel in pixels.chunks_exact_mut(4) {
        texel.copy_from_slice(&BACKGROUND);
    }
    if board.page == Page::Ballot {
        return render_ballot(board, here, ballot, pixels);
    }

    let margin = 6i32;
    let mut y = margin;
    let heading = format!("{} BEACON", here.name);
    font::draw_text(&mut pixels, BOARD_WIDTH, margin, y, 1, ACCENT, &heading);
    let credits = format!("CR {}", walletbook.credits());
    font::draw_text(
        &mut pixels,
        BOARD_WIDTH,
        BOARD_WIDTH as i32 - margin - font::text_width(&credits, 1) as i32,
        y,
        1,
        GOOD,
        &credits,
    );
    y += LINE_HEIGHT as i32;
    let place = format!(
        "{} AT {} {}",
        here.speciality.name(),
        here.centre.0,
        here.centre.1
    );
    font::draw_text(&mut pixels, BOARD_WIDTH, margin, y, 1, DIM, &place);
    y += LINE_HEIGHT as i32;

    // What this town is paying, and what it is short of. The reason to walk in
    // here rather than sell everything at home.
    for (good, name) in economy::GOODS.iter().enumerate() {
        let short = if market.wants(good) { " WANTED" } else { "" };
        let line = format!(
            "{} {} CR - HAS {}{short}",
            crate::shop::display_name(name),
            market.price(good),
            market.stock(good).round() as u64
        );
        let colour = if market.wants(good) { ACCENT } else { DIM };
        font::draw_text(&mut pixels, BOARD_WIDTH, margin, y, 1, colour, &line);
        y += LINE_HEIGHT as i32;
    }
    y += 3;

    // Who runs the place, under what it pays. A town has been a price list
    // with a name on it for thirty rounds; this is the line that says there
    // are people behind the counter.
    for (title, name) in &civic.seats {
        font::draw_text(
            &mut pixels,
            BOARD_WIDTH,
            margin,
            y,
            1,
            DIM,
            &format!("{title:<8} {name}"),
        );
        y += LINE_HEIGHT as i32;
    }
    if let Some(charter) = &civic.charter {
        font::draw_text(&mut pixels, BOARD_WIDTH, margin, y, 1, DIM, charter);
        y += LINE_HEIGHT as i32;
    }
    if let Some(warrant) = &civic.warrant {
        font::draw_text(&mut pixels, BOARD_WIDTH, margin, y, 1, WARN, warrant);
        y += LINE_HEIGHT as i32;
    }
    if civic.closed {
        font::draw_text(
            &mut pixels,
            BOARD_WIDTH,
            margin,
            y,
            1,
            WARN,
            "THIS TOWN WILL NOT TRADE WITH YOU",
        );
        y += LINE_HEIGHT as i32;
    }
    y += 3;

    let rows = Board::rows_with_runs(here.centre, postings, ledger, runs);
    if rows.is_empty() {
        font::draw_text(&mut pixels, BOARD_WIDTH, margin, y, 1, DIM, "NO WORK POSTED");
        y += LINE_HEIGHT as i32;
    }
    for (index, row) in rows.iter().enumerate() {
        let selected = index == board.cursor;
        if selected {
            font::draw_text(&mut pixels, BOARD_WIDTH, margin, y, 1, ACCENT, ">");
        }
        // A trade run is not a posting; it draws its own line and moves on.
        if let Row::Ship { good, to } = row {
            let name = runs
                .iter()
                .find(|run| run.good == *good && run.to == *to)
                .map_or_else(|| format!("{} {}", to.0, to.1), |run| run.name.clone());
            let label = format!(
                "SHIP {} TO {name}",
                crate::shop::display_name(economy::GOODS[*good])
            );
            let colour = if selected { GOOD } else { DIM };
            font::draw_text(&mut pixels, BOARD_WIDTH, margin + 10, y, 1, colour, &label);
            y += LINE_HEIGHT as i32;
            let pay = format!("{} A LOAD, PAID ON ARRIVAL", market.price(*good));
            font::draw_text(&mut pixels, BOARD_WIDTH, margin + 18, y, 1, DIM, &pay);
            y += LINE_HEIGHT as i32;
            continue;
        }

        let Some(id) = row.id() else { continue };
        let posting = postings
            .iter()
            .find(|posting| posting.id == id)
            .or_else(|| ledger.get(id));
        let Some(posting) = posting else { continue };

        let (prefix, colour) = match row {
            // Drawn above and skipped; kept exhaustive rather than unreachable.
            Row::Ship { .. } => ("SHIP", GOOD),
            Row::HandIn(_) => ("SIGN", GOOD),
            Row::Accept(_) => ("TAKE", if selected { TEXT } else { DIM }),
            Row::Taken(_) => ("HELD", DIM),
        };
        let label = format!("{prefix} {}", posting.title());
        font::draw_text(&mut pixels, BOARD_WIDTH, margin + 10, y, 1, colour, &label);
        y += LINE_HEIGHT as i32;
        // Where it settles, and whether the player has any idea where that
        // is. "UNCHARTED" is the hook: the pin is on the map, the ground
        // around it is not.
        let settles = posting.settles_at();
        let place = if ledger.knows(settles) {
            format!("{} {}", settles.0, settles.1)
        } else {
            "UNCHARTED".to_string()
        };
        let pay = format!("{} CR - {place}", posting.reward);
        font::draw_text(&mut pixels, BOARD_WIDTH, margin + 18, y, 1, DIM, &pay);
        y += LINE_HEIGHT as i32;
    }

    if let Some(feedback) = &board.feedback {
        y += 3;
        font::draw_text(&mut pixels, BOARD_WIDTH, margin, y, 1, GOOD, feedback);
        y += LINE_HEIGHT as i32;
    }

    // The trade map. Centred midway between here and wherever the selected run
    // is going, zoomed to fit both, and black wherever you have not walked —
    // which needs no special handling, because the map's marker pass has never
    // had a visibility test. A destination you have never seen is a pin sitting
    // in the dark, which is the whole point.
    if let (Some(view), Some(Row::Ship { to, .. })) = (
        view,
        board
            .selected(&rows)
            .filter(|row| matches!(row, Row::Ship { .. })),
    ) {
        y += 4;
        let centre = (
            (here.centre.0 + to.0) / 2,
            (here.centre.1 + to.1) / 2,
        );
        // Enough zoom that both ends sit inside the inset, with a margin.
        // Rounded *up*: truncating here would leave the destination a few
        // blocks off the edge of its own map, which is the one thing this
        // inset exists to show.
        let span = (here.centre.0 - to.0)
            .abs()
            .max((here.centre.1 - to.1).abs());
        let across = span * 5 / 4;
        let zoom = ((across + TRADE_MAP as i32 - 1) / TRADE_MAP as i32).max(1);

        let mut markers = vec![
            Marker {
                x: here.centre.0,
                z: here.centre.1,
                colour: map::colour::TOWN,
                radius: 2,
            },
            Marker {
                x: to.0,
                z: to.1,
                colour: map::colour::CONTRACT,
                radius: 3,
            },
        ];
        for (x, z) in view.traffic {
            markers.push(Marker {
                x: *x,
                z: *z,
                colour: map::colour::TRADE,
                radius: 1,
            });
        }

        let inset = map::render_map_sized(
            view.world,
            view.explored,
            centre,
            zoom,
            TRADE_MAP,
            &markers,
        );
        let left = (BOARD_WIDTH as i32 - TRADE_MAP as i32) / 2;
        map::blit(&mut pixels, BOARD_WIDTH, &inset, TRADE_MAP, left, y);

        map::frame(&mut pixels, BOARD_WIDTH, TRADE_MAP, left, y, DIM);
        y += TRADE_MAP as i32 + 3;

        let name = runs
            .iter()
            .find(|run| run.to == to)
            .map_or_else(|| format!("{} {}", to.0, to.1), |run| run.name.clone());
        let line = format!("{name} - {}", map::bearing(here.centre, to));
        font::draw_text(
            &mut pixels,
            BOARD_WIDTH,
            (BOARD_WIDTH as i32 - font::text_width(&line, 1) as i32) / 2,
            y,
            1,
            ACCENT,
            &line,
        );
    }

    font::draw_text(
        &mut pixels,
        BOARD_WIDTH,
        margin,
        BOARD_HEIGHT as i32 - LINE_HEIGHT as i32 - 2,
        1,
        DIM,
        "ARROWS. ENTER ACTS. TAB VOTES. E LEAVES.",
    );
    pixels
}

/// The rows the ballot page offers: one per seat, to stand or to withdraw.
pub fn ballot_rows(ballot: &Ballot) -> Vec<String> {
    ballot
        .seats
        .iter()
        .map(|seat| {
            let verb = if seat.yours {
                "RESIGN THE"
            } else if seat.standing {
                "WITHDRAW FROM THE"
            } else {
                "STAND FOR"
            };
            format!("{verb} {} SEAT", seat.title)
        })
        .collect()
}

/// The ballot page.
///
/// Pure over its snapshot, exactly as the work page is — which is what lets
/// an election be photographed and diffed without a town anywhere near it.
fn render_ballot(board: &Board, here: &TownSite, ballot: &Ballot, mut pixels: Vec<u8>) -> Vec<u8> {
    let margin = 6i32;
    let mut y = margin;
    font::draw_text(
        &mut pixels,
        BOARD_WIDTH,
        margin,
        y,
        1,
        ACCENT,
        &format!("{} BALLOT", here.name),
    );
    y += LINE_HEIGHT as i32;

    let when = match ballot.days_to_poll {
        0 => "POLLING TODAY".to_string(),
        1 => "POLLS TOMORROW".to_string(),
        days => format!("POLLS IN {days} DAYS"),
    };
    font::draw_text(&mut pixels, BOARD_WIDTH, margin, y, 1, DIM, &when);
    y += LINE_HEIGHT as i32 + 3;

    for seat in &ballot.seats {
        let mark = if seat.yours { "YOU" } else { "" };
        let colour = if seat.yours { GOOD } else { TEXT };
        font::draw_text(
            &mut pixels,
            BOARD_WIDTH,
            margin,
            y,
            1,
            colour,
            &format!("{:<8} {:<22} {mark}", seat.title, seat.holder),
        );
        y += LINE_HEIGHT as i32;
    }
    y += 3;

    font::draw_text(&mut pixels, BOARD_WIDTH, margin, y, 1, DIM, "HOW THEY LEAN");
    y += LINE_HEIGHT as i32;
    if ballot.leanings.is_empty() {
        font::draw_text(&mut pixels, BOARD_WIDTH, margin, y, 1, DIM, "NOBODY IS SAYING");
        y += LINE_HEIGHT as i32;
    }
    for line in &ballot.leanings {
        font::draw_text(&mut pixels, BOARD_WIDTH, margin, y, 1, TEXT, line);
        y += LINE_HEIGHT as i32;
    }
    y += 3;

    for (index, row) in ballot_rows(ballot).iter().enumerate() {
        let selected = index == board.cursor;
        if selected {
            font::draw_text(&mut pixels, BOARD_WIDTH, margin, y, 1, ACCENT, ">");
        }
        let colour = if selected { ACCENT } else { TEXT };
        font::draw_text(&mut pixels, BOARD_WIDTH, margin + 10, y, 1, colour, row);
        y += LINE_HEIGHT as i32;
    }

    if let Some(feedback) = &board.feedback {
        y += 3;
        font::draw_text(&mut pixels, BOARD_WIDTH, margin, y, 1, ACCENT, feedback);
    }

    font::draw_text(
        &mut pixels,
        BOARD_WIDTH,
        margin,
        BOARD_HEIGHT as i32 - LINE_HEIGHT as i32 - 2,
        1,
        DIM,
        "ARROWS. ENTER STANDS. TAB BACK. E LEAVES.",
    );
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::beacon::postings_for;
    use vx_world::town;

    fn frontier() -> Vec<vx_world::town::TownSite> {
        town::towns_near(7, (0, 0), 2_000, &|_, _| 90)
    }

    /// The hometown's board, and a town it posts freight to.
    fn home_board() -> (vx_world::town::TownSite, Vec<crate::beacon::Posting>) {
        let home = town::home_site();
        let postings = postings_for(&home, &frontier());
        (home, postings)
    }

    /// A town whose board carries the kind of work a test needs. Boards are
    /// derived, so which town that is is a property of the seed, not
    /// something a fixture gets to choose.
    fn board_with(
        wanted: fn(&crate::beacon::Posting) -> bool,
    ) -> (
        vx_world::town::TownSite,
        Vec<crate::beacon::Posting>,
        crate::beacon::Posting,
    ) {
        let frontier = frontier();
        for site in std::iter::once(town::home_site()).chain(frontier.iter().copied()) {
            let postings = postings_for(&site, &frontier);
            if let Some(found) = postings.iter().find(|posting| wanted(posting)).cloned() {
                return (site, postings, found);
            }
        }
        panic!("no town on the fixture frontier posts that kind of work");
    }

    fn is_freight(posting: &crate::beacon::Posting) -> bool {
        matches!(posting.task, Task::Deliver { .. })
    }

    fn is_survey(posting: &crate::beacon::Posting) -> bool {
        matches!(posting.task, Task::Survey { .. })
    }

    fn never(_: (i32, i32)) -> bool {
        false
    }

    fn always(_: (i32, i32)) -> bool {
        true
    }

    #[test]
    fn a_fresh_board_offers_everything_and_nothing_is_due() {
        let (home, postings) = home_board();
        let ledger = Ledger::new();
        let rows = Board::rows(home.centre, &postings, &ledger);
        assert_eq!(rows.len(), postings.len());
        assert!(rows.iter().all(|row| matches!(row, Row::Accept(_))));
    }

    #[test]
    fn taking_freight_pins_it_and_it_is_not_due_where_you_took_it() {
        let (home, postings, haul) = board_with(is_freight);
        let mut ledger = Ledger::new();
        let mut board = Board::new();
        board.open_at_beacon();

        // Put the cursor on the haul.
        let index = postings.iter().position(|posting| posting.id == haul.id).unwrap();
        board.move_cursor(index as i32, postings.len());

        let mut wallet = Wallet::new();
        board.confirm(&home, &postings, &mut ledger, None, &mut wallet, &never);

        assert!(ledger.is_accepted(haul.id), "the haul was not taken");
        assert!(ledger.pins().contains(&haul.settles_at()));

        // Back at the issuing town it now reads as held, not as due.
        let rows = Board::rows(home.centre, &postings, &ledger);
        assert!(rows.contains(&Row::Taken(haul.id)));
        assert!(!rows.contains(&Row::HandIn(haul.id)));
        assert!(ledger.due_at(home.centre).iter().all(|due| due.id != haul.id));
    }

    #[test]
    fn freight_pays_only_at_the_far_end_and_only_with_the_goods_in_hand() {
        let (home, postings, haul) = board_with(is_freight);
        let Task::Deliver { goods, amount, target } = haul.task.clone() else {
            unreachable!()
        };
        let there = town::TownSite {
            centre: target.centre,
            ..home
        };

        let mut ledger = Ledger::new();
        ledger.accept(&haul);
        let mut wallet = Wallet::new();
        let mut board = Board::new();
        board.open_at_beacon();

        // Short: nothing is paid and nothing leaves the pile.
        let mut pile = Stockpile::new();
        pile.add(&goods, amount - 1);
        board.confirm(&there, &postings, &mut ledger, Some(&mut pile), &mut wallet, &never);
        assert_eq!(wallet.credits(), 0, "an unfulfilled haul paid out");
        assert_eq!(pile.count(&goods), amount - 1, "a refused haul took goods anyway");
        assert!(ledger.is_accepted(haul.id));

        // Enough, plus spare: exactly the contracted amount leaves.
        pile.add(&goods, 5);
        board.confirm(&there, &postings, &mut ledger, Some(&mut pile), &mut wallet, &never);
        assert_eq!(wallet.credits(), haul.reward);
        assert_eq!(pile.count(&goods), 4, "the wrong amount left the pile");
        assert!(ledger.is_settled(haul.id));
        assert!(!ledger.is_accepted(haul.id));

        // And it cannot be signed for twice.
        pile.add(&goods, amount);
        board.confirm(&there, &postings, &mut ledger, Some(&mut pile), &mut wallet, &never);
        assert_eq!(wallet.credits(), haul.reward, "a haul paid out twice");
    }

    #[test]
    fn a_survey_pays_once_the_sector_is_swept_and_not_before() {
        let (home, postings, job) = board_with(is_survey);
        let mut ledger = Ledger::new();
        ledger.accept(&job);
        let mut wallet = Wallet::new();
        let mut board = Board::new();
        board.open_at_beacon();

        board.confirm(&home, &postings, &mut ledger, None, &mut wallet, &never);
        assert_eq!(wallet.credits(), 0, "an unswept sector paid out");
        assert!(ledger.is_accepted(job.id));

        board.confirm(&home, &postings, &mut ledger, None, &mut wallet, &always);
        assert_eq!(wallet.credits(), job.reward);
        assert!(ledger.is_settled(job.id));
    }

    #[test]
    fn work_due_here_is_listed_first() {
        let (home, postings, job) = board_with(is_survey);
        let mut ledger = Ledger::new();
        ledger.accept(&job);
        let rows = Board::rows(home.centre, &postings, &ledger);
        assert_eq!(rows.first(), Some(&Row::HandIn(job.id)));
        // And it is listed once, not once as due and once as held.
        assert_eq!(
            rows.iter().filter(|row| row.id() == Some(job.id)).count(),
            1,
            "a posting was listed twice"
        );
    }

    #[test]
    fn an_empty_board_reports_rather_than_panics() {
        let home = town::home_site();
        let mut ledger = Ledger::new();
        let mut wallet = Wallet::new();
        let mut board = Board::new();
        board.open_at_beacon();
        board.confirm(&home, &[], &mut ledger, None, &mut wallet, &never);
        assert_eq!(board.feedback.as_deref(), Some("NO WORK POSTED"));
    }

    #[test]
    fn the_cursor_clamps_to_the_board() {
        let mut board = Board::new();
        board.move_cursor(9, 3);
        assert_eq!(board.cursor, 2);
        board.move_cursor(-9, 3);
        assert_eq!(board.cursor, 0);
        board.move_cursor(1, 0);
        assert_eq!(board.cursor, 0);
    }

    #[test]
    fn the_panel_renders_deterministically_and_shows_uncharted_targets() {
        let (home, postings, haul) = board_with(is_freight);
        let mut board = Board::new();
        board.open_at_beacon();
        let wallet = Wallet::new();
        let market = economy::Economy::new().market(&home, 0).clone();

        let mut ledger = Ledger::new();
        ledger.accept(&haul);

        let a = render_board(&board, &Counter { here: &home, postings: &postings, runs: &[], market: &market, civic: &Civic::default(), ballot: &Ballot::default() }, &ledger, &wallet, None);
        let b = render_board(&board, &Counter { here: &home, postings: &postings, runs: &[], market: &market, civic: &Civic::default(), ballot: &Ballot::default() }, &ledger, &wallet, None);
        assert_eq!(a.len(), (BOARD_WIDTH * BOARD_HEIGHT * 4) as usize);
        assert_eq!(a, b);

        // Finding the target town changes the panel: "UNCHARTED" becomes a
        // pair of coordinates.
        assert!(!ledger.knows(haul.settles_at()));
        ledger.visit(haul.settles_at());
        let found = render_board(&board, &Counter { here: &home, postings: &postings, runs: &[], market: &market, civic: &Civic::default(), ballot: &Ballot::default() }, &ledger, &wallet, None);
        assert_ne!(a, found, "discovering the target left the board unchanged");
    }

    #[test]
    fn every_word_the_board_prints_is_one_the_font_can_draw() {
        // A character the font has never heard of draws as a filled box, and
        // a panel full of boxes reads as a broken game.
        let (home, postings, haul) = board_with(is_freight);
        let mut ledger = Ledger::new();
        let mut wallet = Wallet::new();
        let mut board = Board::new();
        board.open_at_beacon();

        let mut said: Vec<String> = vec![
            format!("{} BEACON", home.name),
            format!("{} AT {} {}", home.speciality.name(), home.centre.0, home.centre.1),
            "NO WORK POSTED".to_string(),
            "ARROWS PICK. ENTER ACTS. E LEAVES.".to_string(),
        ];
        for posting in &postings {
            said.push(posting.title());
            said.push(format!("{} CR - UNCHARTED", posting.reward));
        }

        // And every line of feedback the console can produce.
        for surveyed in [&never as &dyn Fn((i32, i32)) -> bool, &always] {
            for cursor in 0..postings.len() {
                board.move_cursor(cursor as i32 - board.cursor as i32, postings.len());
                let mut pile = Stockpile::new();
                pile.add("engine:stone", 1);
                board.confirm(
                    &home,
                    &postings,
                    &mut ledger,
                    Some(&mut pile),
                    &mut wallet,
                    surveyed,
                );
                if let Some(line) = &board.feedback {
                    said.push(line.clone());
                }
            }
        }
        ledger.accept(&haul);
        board.confirm(&home, &postings, &mut ledger, None, &mut wallet, &never);
        if let Some(line) = &board.feedback {
            said.push(line.clone());
        }

        for line in said {
            for character in line.chars() {
                assert!(
                    font::knows(character),
                    "the font cannot draw {character:?} in {line:?}"
                );
            }
        }
    }

    #[test]
    fn a_trade_run_takes_the_goods_and_puts_them_in_the_air() {
        // The player's side of the network: goods leave the base pile now and
        // are paid for at the far end on arrival, which is what makes a run a
        // journey rather than a transaction.
        let home = town::home_site();
        let away = frontier()
            .into_iter()
            .find(|site| site.centre != home.centre)
            .expect("the fixture frontier has only one town");

        let mut economy = crate::economy::Economy::new();
        let mut board = Board::new();
        board.open_at_beacon();

        let mut pile = Stockpile::new();
        pile.add("engine:copper_ore", crate::economy::PLAYER_LOAD * 2);

        board.ship(
            &home,
            crate::economy::ORE,
            away.centre,
            Some(&mut pile),
            &mut economy,
            0,
        );

        assert_eq!(
            pile.count("engine:copper_ore"),
            crate::economy::PLAYER_LOAD,
            "a run took the wrong amount out of the pile"
        );
        assert_eq!(economy.shipments().len(), 1, "nothing went into the air");
        let load = &economy.shipments()[0];
        assert_eq!(load.owner, crate::economy::Owner::Player);
        assert_eq!(load.from, home.centre);
        assert_eq!(load.to, away.centre);
        assert!(load.arrive > load.depart, "a run arrived before it left");
    }

    #[test]
    fn a_run_with_nothing_to_carry_takes_nothing() {
        let home = town::home_site();
        let away = frontier()
            .into_iter()
            .find(|site| site.centre != home.centre)
            .unwrap();
        let mut economy = crate::economy::Economy::new();
        let mut board = Board::new();
        board.open_at_beacon();

        // Short of a full load: nothing moves, and it says why.
        let mut pile = Stockpile::new();
        pile.add("engine:copper_ore", crate::economy::PLAYER_LOAD - 1);
        board.ship(
            &home,
            crate::economy::ORE,
            away.centre,
            Some(&mut pile),
            &mut economy,
            0,
        );
        assert_eq!(pile.count("engine:copper_ore"), crate::economy::PLAYER_LOAD - 1);
        assert!(economy.shipments().is_empty(), "an empty run went out");
        assert!(board.feedback.is_some(), "a refused run said nothing");

        // And with no base at all it reports rather than panics.
        board.ship(&home, crate::economy::ORE, away.centre, None, &mut economy, 0);
        assert_eq!(board.feedback.as_deref(), Some("NO BASE PILE"));
    }

    #[test]
    fn selecting_a_trade_run_draws_its_map_and_bearing() {
        // The console's map only appears for a run, because that is the only
        // row that has somewhere to point at.
        let home = town::home_site();
        let away = frontier()
            .into_iter()
            .find(|site| site.centre != home.centre)
            .expect("the fixture frontier has only one town");
        let runs = vec![Run {
            good: economy::ORE,
            to: away.centre,
            name: away.name.to_string(),
        }];

        let world = World::new(2024);
        let explored = MapState::new();
        let view = TradeView {
            world: &world,
            explored: &explored,
            traffic: &[],
        };

        let market = economy::Economy::new().market(&home, 0).clone();
        let ledger = Ledger::new();
        let wallet = Wallet::new();
        let mut board = Board::new();
        board.open_at_beacon();

        // Put the cursor on the run, which is the last row.
        let rows = Board::rows_with_runs(home.centre, &[], &ledger, &runs);
        assert!(matches!(rows.last(), Some(Row::Ship { .. })));
        board.move_cursor(rows.len() as i32, rows.len());

        let civic = Civic::default();
        let ballot = Ballot::default();
        let counter = Counter {
            here: &home,
            postings: &[],
            runs: &runs,
            market: &market,
            civic: &civic,
            ballot: &ballot,
        };
        let with_map = render_board(&board, &counter, &ledger, &wallet, Some(&view));
        let without = render_board(&board, &counter, &ledger, &wallet, None);
        assert_ne!(with_map, without, "the trade map did not draw");
        assert_eq!(
            with_map.len(),
            (BOARD_WIDTH * BOARD_HEIGHT * 4) as usize,
            "the panel changed size"
        );

        // Deterministic, like every other panel.
        assert_eq!(
            with_map,
            render_board(&board, &counter, &ledger, &wallet, Some(&view))
        );

        // And the civic block really draws: a town with a mayor, a sheriff
        // and paper out on you does not look like one with none of that.
        let civic = Civic {
            seats: vec![
                ("MAYOR".into(), "GRANT THE MEEK".into()),
                ("SHERIFF".into(), "HOLLIS THE GRIM".into()),
            ],
            charter: None,
            warrant: Some("WARRANT PETITIONED - FILED ON 140 CR".into()),
            closed: true,
        };
        let civic_counter = Counter {
            here: &home,
            postings: &[],
            runs: &runs,
            market: &market,
            civic: &civic,
            ballot: &ballot,
        };
        let governed = render_board(&board, &civic_counter, &ledger, &wallet, Some(&view));
        assert_ne!(governed, with_map, "the civic block drew nothing at all");
        assert_eq!(governed.len(), with_map.len(), "the panel changed size");

        // And the ballot page is a different page, not a longer column.
        let papers = Ballot {
            seats: vec![
                BallotSeat {
                    title: "MAYOR".into(),
                    holder: "GRANT THE MEEK".into(),
                    yours: false,
                    standing: true,
                },
                BallotSeat {
                    title: "SHERIFF".into(),
                    holder: "YOU".into(),
                    yours: true,
                    standing: false,
                },
            ],
            days_to_poll: 2,
            leanings: vec!["HOLLIS THE GRIM       WOULD VOTE FOR YOU".into()],
        };
        let voting = Counter {
            here: &home,
            postings: &[],
            runs: &runs,
            market: &market,
            civic: &civic,
            ballot: &papers,
        };
        let mut turned = Board::new();
        turned.open_at_beacon();
        turned.turn_page();
        assert_eq!(turned.page, Page::Ballot);
        let page = render_board(&turned, &voting, &ledger, &wallet, Some(&view));
        assert_ne!(page, governed, "the ballot page drew the work page");
        assert_eq!(page.len(), governed.len(), "the panel changed size");
        // Deterministic, like every other panel here.
        assert_eq!(
            page,
            render_board(&turned, &voting, &ledger, &wallet, Some(&view))
        );
        // One row per seat, worded for what pressing Enter would do.
        let rows = ballot_rows(&papers);
        assert_eq!(rows.len(), 2);
        assert!(rows[0].starts_with("WITHDRAW"), "{}", rows[0]);
        assert!(rows[1].starts_with("RESIGN"), "{}", rows[1]);
        // And turning back is turning back.
        turned.turn_page();
        assert_eq!(turned.page, Page::Work);

        // The destination is unexplored here, so it is a pin in the dark —
        // which is exactly the paper-map behaviour asked for.
        assert!(!explored.is_explored(
            vx_core::BlockPos::new(away.centre.0, 0, away.centre.1).chunk()
        ));
    }
}
