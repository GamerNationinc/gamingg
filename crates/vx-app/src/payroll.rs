//! What the operation is worth: the numbers behind "maximum credits".
//!
//! # Why this exists
//!
//! Every figure here was already computable and nothing asked for it. The
//! player could see credits in the wallet and a pile in a container, and had
//! no way at all to answer the three questions the game is actually about:
//!
//! - **What is the crew earning me?** Blocks an hour, which is the only
//!   number that says whether a second drone was worth 375 credits.
//! - **What is this pile worth?** Which is not "how many blocks is it" — a
//!   barrow of stone and a barrow of uranium are the same barrow.
//! - **Where is it worth most?** Since stage 52 that has a second half: not
//!   just which counter pays the best rate, but which counter has the money.
//!
//! The last one is the round's whole design change. A town pays out of a till
//! that its own trade refills ([`crate::economy`]), so a rich town at a fair
//! price can beat a desperate one that is skint, and a counter you emptied
//! yesterday is one to walk past today. That is a routing problem, and a
//! routing problem you cannot see the inputs to is a guess.
//!
//! # Pure, like every other panel here
//!
//! Functions over snapshots. Nothing in this file reads the clock, touches the
//! world or mutates anything, so the terminal, the HUD and the played session
//! all quote the same arithmetic — and the figure a player is shown is the
//! figure the till actually pays, because both go through
//! [`crate::economy::Market::affords`].

use vx_agent::Stockpile;
use vx_world::town::TownSite;

use crate::economy::{self, Economy, Market};
use crate::reputation::Standing;

/// What one counter would give you for a pile, and what it can actually pay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Quote {
    /// What the whole pile is worth at this counter's prices.
    pub worth: u64,
    /// What this counter can pay today, which is `worth` capped by its till.
    pub paid: u64,
}

impl Quote {
    /// Is the counter short of money for what you are carrying?
    pub fn short(&self) -> bool {
        self.paid < self.worth
    }
}

/// What a pile fetches at one counter.
///
/// Exact rather than an estimate, and it has to be: [`crate::shop::sell_all`]
/// prices a whole stack at the board rate *before* the sale lands, so the
/// worth is a plain sum. The `paid` half walks the goods in the order the shelf
/// sells them and draws the till down as it goes, which is what a player
/// clicking down the list would actually get.
pub fn quote(pile: &Stockpile, market: &Market, standing: Standing) -> Quote {
    let mut worth = 0u64;
    let mut paid = 0u64;
    let mut till = market.till();
    for (name, count) in pile.entries() {
        let Some(good) = economy::good_index(name) else {
            continue;
        };
        let rate = crate::reputation::shaded_sell(market.price(good), standing);
        worth = worth.saturating_add(count.saturating_mul(rate));
        // What this counter can still afford of this stack, at this rate.
        let affordable = count.min(if rate == 0 { 0 } else { till / rate });
        let taken = affordable.saturating_mul(rate);
        paid = paid.saturating_add(taken);
        till -= taken;
    }
    Quote { worth, paid }
}

/// The best counter for a pile among `sites`, and what it would pay.
///
/// Ranked on what is actually **paid**, not on the sticker price: a refinery
/// that would love your ore and has forty credits left is worth less than a
/// depot that merely likes it and has a full till. That ordering is the point
/// of the round.
///
/// Ties break on the town's own coordinates so the answer is the same every
/// run — this is quoted into the terminal and into a played session, and an
/// answer that wandered would make both meaningless.
pub fn best_counter(
    pile: &Stockpile,
    sites: &[TownSite],
    books: &mut Economy,
    now: u64,
    standing: Standing,
) -> Option<(TownSite, Quote)> {
    sites
        .iter()
        .map(|site| {
            let quote = quote(pile, books.market(site, now), standing);
            (*site, quote)
        })
        .max_by_key(|(site, quote)| (quote.paid, quote.worth, site.centre.0, site.centre.1))
}

/// What a crew has been cutting, over a window.
///
/// A rolling pair rather than a running average: blocks and the ticks they
/// took, so the caller can widen or reset the window without the number
/// dragging its own history behind it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Rate {
    pub blocks: u64,
    pub ticks: u64,
}

/// Journal ticks in an hour of game time. The journal runs at eight a second.
pub const TICKS_PER_HOUR: u64 = 8 * 60 * 60;

impl Rate {
    /// Add a slice of work.
    pub fn record(&mut self, blocks: u64, ticks: u64) {
        self.blocks = self.blocks.saturating_add(blocks);
        self.ticks = self.ticks.saturating_add(ticks);
    }

    /// Blocks an hour, or `None` before anything has been measured.
    ///
    /// `None` rather than zero, because "no crew has worked yet" and "the crew
    /// is achieving nothing" are different things and the readout says so.
    pub fn per_hour(&self) -> Option<u64> {
        if self.ticks == 0 {
            return None;
        }
        Some(self.blocks.saturating_mul(TICKS_PER_HOUR) / self.ticks)
    }
}

/// The lines the `PAYROLL` verb prints, and the HUD's one-liner.
///
/// Built here rather than in `main` so the played session and the game quote
/// the same words as well as the same numbers.
pub fn lines(
    crew: u32,
    rate: Rate,
    pile: &Stockpile,
    here: Option<(&TownSite, Quote)>,
    best: Option<(&TownSite, Quote)>,
) -> Vec<String> {
    let mut out = Vec::new();
    out.push(match rate.per_hour() {
        Some(hourly) => format!("CREW {crew} - {hourly} BLOCKS/HR"),
        None => format!("CREW {crew} - NOT WORKING"),
    });
    out.push(format!("PILE {} GOODS", pile.total()));
    match here {
        Some((_, quote)) if quote.short() => out.push(format!(
            "HERE {} CR - THIS COUNTER CAN PAY {}",
            quote.worth, quote.paid
        )),
        Some((_, quote)) => out.push(format!("HERE {} CR", quote.worth)),
        None => out.push("HERE NO COUNTER".to_string()),
    }
    if let Some((site, quote)) = best {
        let (x, z) = site.centre;
        let away = ((f64::from(x)).hypot(f64::from(z))).round() as i64;
        out.push(format!(
            "BEST {} {x},{z} - {} CR, {away} BLOCKS",
            site.speciality.name().to_uppercase(),
            quote.paid
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use vx_world::town;

    fn frontier() -> Vec<TownSite> {
        town::towns_near(7, (0, 0), 2_000, &|_, _| 90)
    }

    fn ore(count: u64) -> Stockpile {
        let mut pile = Stockpile::new();
        pile.add("engine:copper_ore", count);
        pile
    }

    /// The readout cannot drift from the till: what it quotes is what selling
    /// actually pays, to the credit.
    #[test]
    fn what_the_readout_quotes_is_what_the_counter_pays() {
        let site = town::home_site();
        let mut books = Economy::new();
        let mut pile = ore(40);
        let quoted = quote(&pile, books.market(&site, 0), Standing::Neutral);

        let mut wallet = crate::wallet::Wallet::new();
        let market = books.market_mut(&site, 0);
        let (_, earned) = crate::shop::sell_all(
            &mut pile,
            &mut wallet,
            market,
            "engine:copper_ore",
            Standing::Neutral,
        );
        assert_eq!(earned, quoted.paid, "the readout lied about the payment");
        assert_eq!(wallet.credits(), quoted.paid);
    }

    /// A pile bigger than the till is quoted as both numbers, so a player can
    /// see the difference between "worth this" and "gets this here".
    #[test]
    fn a_counter_short_of_money_says_so() {
        let site = town::home_site();
        let mut books = Economy::new();
        let hoard = ore(100_000);
        let quoted = quote(&hoard, books.market(&site, 0), Standing::Neutral);
        assert!(quoted.short(), "an enormous pile did not outgrow the till");
        assert!(quoted.paid < quoted.worth);
        assert_eq!(quoted.paid, books.market(&site, 0).till());
    }

    /// The best counter is the one that pays most, not the one that likes your
    /// goods most — a refinery with an empty till loses to a town that can
    /// actually settle up. That ordering is the whole point of the round.
    #[test]
    fn the_best_counter_is_the_one_that_can_pay() {
        let sites = frontier();
        assert!(sites.len() > 2, "the fixture frontier is too small");
        let pile = ore(60);
        let mut books = Economy::new();

        let (best, quote) = best_counter(&pile, &sites, &mut books, 0, Standing::Neutral)
            .expect("no counter at all");
        for site in &sites {
            let theirs = self::quote(&pile, books.market(site, 0), Standing::Neutral);
            assert!(
                theirs.paid <= quote.paid,
                "{:?} would pay {} but {:?} was picked at {}",
                site.centre,
                theirs.paid,
                best.centre,
                quote.paid
            );
        }

        // Drain the winner and it stops being the answer.
        let drained = books.market_mut(&best, 0).till();
        books.market_mut(&best, 0).draw(drained);
        let (after, _) = best_counter(&pile, &sites, &mut books, 0, Standing::Neutral)
            .expect("no counter after draining one");
        assert_ne!(
            after.centre, best.centre,
            "an emptied counter was still the best place to sell"
        );
    }

    /// The same frontier gives the same answer twice: this is quoted into a
    /// played session, and an answer that wandered would make it meaningless.
    #[test]
    fn the_best_counter_is_the_same_answer_every_time() {
        let sites = frontier();
        let pile = ore(60);
        let mut once = Economy::new();
        let mut twice = Economy::new();
        assert_eq!(
            best_counter(&pile, &sites, &mut once, 0, Standing::Neutral).map(|(site, _)| site.centre),
            best_counter(&pile, &sites, &mut twice, 0, Standing::Neutral).map(|(site, _)| site.centre),
        );
    }

    #[test]
    fn a_rate_is_blocks_an_hour_and_says_nothing_before_it_knows() {
        let mut rate = Rate::default();
        assert_eq!(rate.per_hour(), None, "a rate spoke before it measured");
        // Sixty blocks in a quarter of an hour is 240 an hour.
        rate.record(60, TICKS_PER_HOUR / 4);
        assert_eq!(rate.per_hour(), Some(240));
        // And it cannot be made to wrap.
        rate.record(u64::MAX, 1);
        assert!(rate.per_hour().is_some());
    }

    /// Every line the readout prints has to be one the bitmap font can draw,
    /// like every other string that reaches the screen.
    #[test]
    fn every_payroll_line_is_drawable() {
        let sites = frontier();
        let pile = ore(100_000);
        let mut books = Economy::new();
        let here = *sites.first().expect("no towns");
        let here_quote = quote(&pile, books.market(&here, 0), Standing::Neutral);
        let best = best_counter(&pile, &sites, &mut books, 0, Standing::Neutral);
        let printed = lines(
            2,
            Rate {
                blocks: 90,
                ticks: TICKS_PER_HOUR,
            },
            &pile,
            Some((&here, here_quote)),
            best.as_ref().map(|(site, quote)| (site, *quote)),
        );
        assert!(printed.len() >= 4, "the readout said almost nothing");
        for line in &printed {
            for character in line.chars() {
                assert!(
                    vx_render::font::knows(character),
                    "undrawable {character:?} in {line:?}"
                );
            }
        }
        // The short-till line is the one worth pinning: it is the whole point.
        assert!(
            printed.iter().any(|line| line.contains("CAN PAY")),
            "a counter short of money did not say so: {printed:#?}"
        );
    }
}
