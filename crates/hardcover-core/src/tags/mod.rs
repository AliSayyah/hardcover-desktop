//! Hardcover's community tags: the moods, pace and character tags readers
//! vote for on a book, the rule that turns those votes into a book's mood
//! line, and genres merged across their many spellings.
//!
//! Hardcover's review form asks a reader for moods, a pace and a few
//! character tags; each answer is a vote, and `taggable_counts` holds each
//! book's exact count per tag (verified live, read-only, 2026-11-23).
//! `cached_tags` only keeps the top few of each, rounded off, so the book
//! page and the shelf filter ask for the counts themselves
//! ([`VOTE_TAG_IDS`], [`MOOD_IDS`]).
//!
//! Only the 14 [`Mood`]s count as moods: Hardcover has many more mood tags,
//! but below these 14 their use falls off a cliff and they are noise.
//!
//! A mood line names what is distinctive about a book, not what is common
//! to every book: a mood stands out when it holds a good share of the
//! book's mood votes *and* more than its share of the site's ([`mood_line`]).
//! The shelf filter uses the same line ([`has_mood`]), so a book filtered
//! by a mood always shows that mood.

mod genre;
#[cfg(test)]
mod tests;

use std::cmp::{Ordering, Reverse};
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub use crate::api::TagId;
pub use genre::{
    BROWSE_GENRES, BookGenres, BrowseGenre, Genre, MIN_GENRE_VOTES, browse_genre, genre_key,
    genre_words,
};

// ----- thresholds -----

/// A book needs this many votes over the 14 moods for a mood line (and a
/// Rarely list) at all.
pub const LINE_MIN_MOOD_VOTES: u32 = 50;
/// A mood stands out with at least this share of the book's mood votes, in
/// percent...
pub const STANDOUT_MIN_SHARE_PERCENT: u32 = 10;
/// ...and a lift of at least this many tenths (1.5): its share of the
/// book's mood votes over its share of the site's ([`Mood::site_votes`]).
pub const STANDOUT_MIN_LIFT_TENTHS: u32 = 15;
/// Most moods a line names.
pub const LINE_MAX_MOODS: usize = 3;
/// A pace is named, and the pace and drive splits are shown, only with at
/// least this many votes in the group.
pub const GROUP_MIN_VOTES: u32 = 20;
/// A mood is Rarely with a lift of at most this many tenths (0.2).
pub const RARELY_MAX_LIFT_TENTHS: u32 = 2;
/// Most moods the Rarely list names.
pub const RARELY_MAX: usize = 3;
/// The breakdown lists a mood with at least this many votes.
pub const BREAKDOWN_MIN_MOOD_VOTES: u32 = 5;
/// A character pair is shown only with at least this many votes on its two
/// sides together...
pub const TRAIT_MIN_PAIR_VOTES: u32 = 20;
/// ...and one side holding at least this share of them, as a fraction
/// (numerator, denominator): two thirds.
pub const TRAIT_MIN_SHARE: (u32, u32) = (2, 3);
/// A tag page narrowed by a second tag lists a book only with at least this
/// many votes for that tag (`Client::books_by_tag`), so one stray vote
/// doesn't put a book on it. Verified live to narrow sensibly (2026-11-23).
pub const MIN_OTHER_TAG_VOTES: u32 = 3;

// ----- the tags -----

/// One of the 14 moods Hardcover's review form offers, the moods that count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Mood {
    Adventurous,
    Emotional,
    Dark,
    Mysterious,
    Tense,
    Funny,
    Reflective,
    Lighthearted,
    Sad,
    Hopeful,
    Challenging,
    Inspiring,
    Informative,
    Relaxing,
}

impl Mood {
    /// Every mood, the most used on the site first.
    pub const ALL: [Mood; 14] = [
        Mood::Adventurous,
        Mood::Emotional,
        Mood::Dark,
        Mood::Mysterious,
        Mood::Tense,
        Mood::Funny,
        Mood::Reflective,
        Mood::Lighthearted,
        Mood::Sad,
        Mood::Hopeful,
        Mood::Challenging,
        Mood::Inspiring,
        Mood::Informative,
        Mood::Relaxing,
    ];

    /// The mood's tag id.
    #[must_use]
    pub const fn id(self) -> TagId {
        TagId::new(match self {
            Mood::Adventurous => 154,
            Mood::Emotional => 191,
            Mood::Dark => 148,
            Mood::Mysterious => 207,
            Mood::Tense => 206,
            Mood::Funny => 180,
            Mood::Reflective => 192,
            Mood::Lighthearted => 181,
            Mood::Sad => 193,
            Mood::Hopeful => 187,
            Mood::Challenging => 190,
            Mood::Inspiring => 240,
            Mood::Informative => 203,
            Mood::Relaxing => 241,
        })
    }

    /// The name in title case, as Hardcover's review form writes it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Mood::Adventurous => "Adventurous",
            Mood::Emotional => "Emotional",
            Mood::Dark => "Dark",
            Mood::Mysterious => "Mysterious",
            Mood::Tense => "Tense",
            Mood::Funny => "Funny",
            Mood::Reflective => "Reflective",
            Mood::Lighthearted => "Lighthearted",
            Mood::Sad => "Sad",
            Mood::Hopeful => "Hopeful",
            Mood::Challenging => "Challenging",
            Mood::Inspiring => "Inspiring",
            Mood::Informative => "Informative",
            Mood::Relaxing => "Relaxing",
        }
    }

    /// The tag's slug: the name in lowercase.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Mood::Adventurous => "adventurous",
            Mood::Emotional => "emotional",
            Mood::Dark => "dark",
            Mood::Mysterious => "mysterious",
            Mood::Tense => "tense",
            Mood::Funny => "funny",
            Mood::Reflective => "reflective",
            Mood::Lighthearted => "lighthearted",
            Mood::Sad => "sad",
            Mood::Hopeful => "hopeful",
            Mood::Challenging => "challenging",
            Mood::Inspiring => "inspiring",
            Mood::Informative => "informative",
            Mood::Relaxing => "relaxing",
        }
    }

    /// The mood's votes over every book on Hardcover (`tags.count`, read
    /// live on 2026-11-22). The base a book's own share is compared with;
    /// it moves slowly, so a constant is close enough.
    #[must_use]
    pub const fn site_votes(self) -> u32 {
        match self {
            Mood::Adventurous => 75_858,
            Mood::Emotional => 75_096,
            Mood::Dark => 68_433,
            Mood::Mysterious => 57_316,
            Mood::Tense => 53_022,
            Mood::Funny => 45_687,
            Mood::Reflective => 44_129,
            Mood::Lighthearted => 36_867,
            Mood::Sad => 33_225,
            Mood::Hopeful => 31_712,
            Mood::Challenging => 28_774,
            Mood::Inspiring => 23_364,
            Mood::Informative => 21_923,
            Mood::Relaxing => 10_452,
        }
    }

    #[must_use]
    pub fn from_id(id: TagId) -> Option<Mood> {
        Mood::ALL.into_iter().find(|m| m.id() == id)
    }

    #[must_use]
    pub fn from_slug(slug: &str) -> Option<Mood> {
        Mood::ALL.into_iter().find(|m| m.slug() == slug)
    }
}

/// Every mood's site votes together: the base of a mood's site share.
pub const SITE_MOOD_VOTES: u64 = {
    let mut sum = 0;
    let mut i = 0;
    while i < Mood::ALL.len() {
        sum += Mood::ALL[i].site_votes() as u64;
        i += 1;
    }
    sum
};

/// The 14 moods' tag ids, the most used first.
pub const MOOD_IDS: [TagId; 14] = {
    let mut ids = [TagId::new(0); 14];
    let mut i = 0;
    while i < ids.len() {
        ids[i] = Mood::ALL[i].id();
        i += 1;
    }
    ids
};

/// How fast a book reads, by its readers' votes. In order on the scale, so a
/// split reads from slow to fast.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Pace {
    Slow,
    Medium,
    Fast,
}

impl Pace {
    pub const ALL: [Pace; 3] = [Pace::Slow, Pace::Medium, Pace::Fast];

    #[must_use]
    pub const fn id(self) -> TagId {
        TagId::new(match self {
            Pace::Slow => 195,
            Pace::Medium => 182,
            Pace::Fast => 188,
        })
    }

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Pace::Slow => "Slow",
            Pace::Medium => "Medium",
            Pace::Fast => "Fast",
        }
    }
}

/// What drives a book, by its readers' votes. In order on the scale, plot
/// to characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Drive {
    Plot,
    Mix,
    Character,
}

impl Drive {
    pub const ALL: [Drive; 3] = [Drive::Plot, Drive::Mix, Drive::Character];

    #[must_use]
    pub const fn id(self) -> TagId {
        TagId::new(match self {
            Drive::Plot => 208,
            Drive::Mix => 183,
            Drive::Character => 194,
        })
    }

    /// The tag's name, as Hardcover writes it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Drive::Plot => "Plot driven",
            Drive::Mix => "A mix driven",
            Drive::Character => "Character driven",
        }
    }
}

/// One side of a [`TraitPair`] on Hardcover's review form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum CharacterTrait {
    Loveable,
    Unloveable,
    StrongDevelopment,
    WeakDevelopment,
    Diverse,
    NotDiverse,
}

impl CharacterTrait {
    pub const ALL: [CharacterTrait; 6] = [
        CharacterTrait::Loveable,
        CharacterTrait::Unloveable,
        CharacterTrait::StrongDevelopment,
        CharacterTrait::WeakDevelopment,
        CharacterTrait::Diverse,
        CharacterTrait::NotDiverse,
    ];

    #[must_use]
    pub const fn id(self) -> TagId {
        TagId::new(match self {
            CharacterTrait::Loveable => 185,
            CharacterTrait::Unloveable => 197,
            CharacterTrait::StrongDevelopment => 184,
            CharacterTrait::WeakDevelopment => 189,
            CharacterTrait::Diverse => 186,
            CharacterTrait::NotDiverse => 196,
        })
    }

    /// The tag's name, as Hardcover writes it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            CharacterTrait::Loveable => "Loveable Characters",
            CharacterTrait::Unloveable => "Unloveable Characters",
            CharacterTrait::StrongDevelopment => "Strong Character Development",
            CharacterTrait::WeakDevelopment => "Weak Character Development",
            CharacterTrait::Diverse => "Diverse Characters",
            CharacterTrait::NotDiverse => "Not Diverse Characters",
        }
    }

    #[must_use]
    pub const fn pair(self) -> TraitPair {
        match self {
            CharacterTrait::Loveable | CharacterTrait::Unloveable => TraitPair::Loveable,
            CharacterTrait::StrongDevelopment | CharacterTrait::WeakDevelopment => {
                TraitPair::Development
            }
            CharacterTrait::Diverse | CharacterTrait::NotDiverse => TraitPair::Diversity,
        }
    }
}

/// Two opposite character tags, of which a reader picks one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum TraitPair {
    Loveable,
    Development,
    Diversity,
}

impl TraitPair {
    pub const ALL: [TraitPair; 3] = [
        TraitPair::Loveable,
        TraitPair::Development,
        TraitPair::Diversity,
    ];

    /// The two sides, the positive one first.
    #[must_use]
    pub const fn sides(self) -> [CharacterTrait; 2] {
        match self {
            TraitPair::Loveable => [CharacterTrait::Loveable, CharacterTrait::Unloveable],
            TraitPair::Development => [
                CharacterTrait::StrongDevelopment,
                CharacterTrait::WeakDevelopment,
            ],
            TraitPair::Diversity => [CharacterTrait::Diverse, CharacterTrait::NotDiverse],
        }
    }
}

/// The tags whose votes a book page asks for: the 14 moods, the 3 paces,
/// the 3 drives and the 6 character tags.
pub const VOTE_TAG_IDS: [TagId; 26] = {
    let mut ids = [TagId::new(0); 26];
    let mut n = 0;
    let mut i = 0;
    while i < Mood::ALL.len() {
        ids[n] = Mood::ALL[i].id();
        n += 1;
        i += 1;
    }
    i = 0;
    while i < Pace::ALL.len() {
        ids[n] = Pace::ALL[i].id();
        n += 1;
        i += 1;
    }
    i = 0;
    while i < Drive::ALL.len() {
        ids[n] = Drive::ALL[i].id();
        n += 1;
        i += 1;
    }
    i = 0;
    while i < CharacterTrait::ALL.len() {
        ids[n] = CharacterTrait::ALL[i].id();
        n += 1;
        i += 1;
    }
    ids
};

// ----- a book's votes -----

/// A book's votes per mood, pace and character tag (`taggable_counts`).
/// A tag with no votes counts 0. Built from whichever tags were asked for:
/// the book page asks for [`VOTE_TAG_IDS`], a shelf's batch only for the
/// moods ([`MOOD_IDS`]), so its votes name no pace.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookVotes {
    counts: BTreeMap<TagId, u32>,
}

impl BookVotes {
    /// The votes from `(tag, count)` pairs. Tags outside [`VOTE_TAG_IDS`]
    /// are left out; a tag listed twice adds up.
    pub fn from_counts(counts: impl IntoIterator<Item = (TagId, u32)>) -> Self {
        let mut votes = Self::default();
        for (tag, count) in counts {
            if count > 0 && VOTE_TAG_IDS.contains(&tag) {
                let n = votes.counts.entry(tag).or_default();
                *n = n.saturating_add(count);
            }
        }
        votes
    }

    /// The votes for `tag`; 0 for a tag outside [`VOTE_TAG_IDS`].
    #[must_use]
    pub fn get(&self, tag: TagId) -> u32 {
        self.counts.get(&tag).copied().unwrap_or(0)
    }

    #[must_use]
    pub fn mood(&self, mood: Mood) -> u32 {
        self.get(mood.id())
    }

    #[must_use]
    pub fn pace(&self, pace: Pace) -> u32 {
        self.get(pace.id())
    }

    #[must_use]
    pub fn drive(&self, drive: Drive) -> u32 {
        self.get(drive.id())
    }

    #[must_use]
    pub fn character(&self, side: CharacterTrait) -> u32 {
        self.get(side.id())
    }

    /// The votes over the 14 moods together.
    #[must_use]
    pub fn mood_votes(&self) -> u32 {
        Mood::ALL.iter().map(|m| self.mood(*m)).sum()
    }

    /// True when no tag has a vote.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.counts.is_empty()
    }
}

// ----- the mood line -----

/// The line under a book's title: up to three moods and maybe a pace.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct MoodLine {
    /// One to [`LINE_MAX_MOODS`] moods: the standouts, the one that stands
    /// out most first, or, with `fallback`, the most voted.
    pub moods: Vec<Mood>,
    /// True when no mood stood out and the line names the most voted ones.
    pub fallback: bool,
    /// Fast or Slow, when readers agree; never Medium.
    pub pace: Option<Pace>,
}

/// A book's mood line, or `None` with fewer than [`LINE_MIN_MOOD_VOTES`]
/// mood votes.
///
/// - A mood stands out with at least [`STANDOUT_MIN_SHARE_PERCENT`] of the
///   book's mood votes and a lift (its share of the book's mood votes over
///   its share of the site's) of at least 1.5. The line names the standouts
///   by lift, at most [`LINE_MAX_MOODS`].
/// - When none stands out, it names the three most voted (`fallback`).
/// - The pace is Fast or Slow when it leads its group, with at least
///   [`GROUP_MIN_VOTES`] pace votes, and holds at least half of them.
///
/// Ties go to the most voted, then the lowest tag id.
#[must_use]
pub fn mood_line(votes: &BookVotes) -> Option<MoodLine> {
    let total = votes.mood_votes();
    if total < LINE_MIN_MOOD_VOTES {
        return None;
    }
    let mut standouts: Vec<Mood> = Mood::ALL
        .into_iter()
        .filter(|m| stands_out(votes.mood(*m), m.site_votes(), total))
        .collect();
    standouts.sort_by(|a, b| by_lift(votes, *a, *b));
    let fallback = standouts.is_empty();
    let mut moods = if fallback {
        by_votes(votes)
            .into_iter()
            .filter(|(_, n)| *n > 0)
            .map(|(m, _)| m)
            .collect()
    } else {
        standouts
    };
    moods.truncate(LINE_MAX_MOODS);
    Some(MoodLine {
        moods,
        fallback,
        pace: named_pace(votes),
    })
}

/// Whether a book has `mood`, for a shelf's mood filter: the mood is on its
/// [`mood_line`], so a book the filter keeps always shows the mood.
#[must_use]
pub fn has_mood(votes: &BookVotes, mood: Mood) -> bool {
    mood_line(votes).is_some_and(|line| line.moods.contains(&mood))
}

/// The moods readers rarely pick for a book: a lift of at most 0.2, the
/// most used on the site first, at most [`RARELY_MAX`]. Empty with fewer
/// than [`LINE_MIN_MOOD_VOTES`] mood votes. A mood with no votes counts.
#[must_use]
pub fn rarely(votes: &BookVotes) -> Vec<Mood> {
    let total = votes.mood_votes();
    if total < LINE_MIN_MOOD_VOTES {
        return Vec::new();
    }
    Mood::ALL
        .into_iter()
        .filter(|m| {
            // count / total <= 0.2 * site / sum
            u128::from(votes.mood(*m)) * u128::from(SITE_MOOD_VOTES) * 10
                <= u128::from(RARELY_MAX_LIFT_TENTHS)
                    * u128::from(m.site_votes())
                    * u128::from(total)
        })
        .take(RARELY_MAX)
        .collect()
}

fn stands_out(count: u32, site: u32, total: u32) -> bool {
    let (count, site, total) = (u128::from(count), u128::from(site), u128::from(total));
    // count / total >= 10%
    let share = count * 100 >= u128::from(STANDOUT_MIN_SHARE_PERCENT) * total;
    // (count / total) / (site / sum) >= 1.5
    let lift = count * u128::from(SITE_MOOD_VOTES) * 10
        >= u128::from(STANDOUT_MIN_LIFT_TENTHS) * site * total;
    share && lift
}

/// The highest lift first; ties by votes, then tag id.
fn by_lift(votes: &BookVotes, a: Mood, b: Mood) -> Ordering {
    let (va, vb) = (votes.mood(a), votes.mood(b));
    // va / site_a against vb / site_b
    let lift_a = u64::from(va) * u64::from(b.site_votes());
    let lift_b = u64::from(vb) * u64::from(a.site_votes());
    lift_b
        .cmp(&lift_a)
        .then(vb.cmp(&va))
        .then(a.id().cmp(&b.id()))
}

/// Every mood with its votes, the most voted first, then by tag id.
fn by_votes(votes: &BookVotes) -> Vec<(Mood, u32)> {
    let mut moods: Vec<(Mood, u32)> = Mood::ALL.into_iter().map(|m| (m, votes.mood(m))).collect();
    moods.sort_by_key(|(m, n)| (Reverse(*n), m.id()));
    moods
}

fn named_pace(votes: &BookVotes) -> Option<Pace> {
    let split = Split::of(Pace::ALL, |p| votes.pace(p), Pace::id)?;
    let lead = split.leader;
    (lead != Pace::Medium && split.votes(lead) * 2 >= split.total).then_some(lead)
}

// ----- the breakdown -----

/// A group's votes split between its tags (the pace, the drive), in the
/// group's own order on the scale, zeros included.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Split<T> {
    pub parts: Vec<(T, u32)>,
    pub total: u32,
    /// The most voted part; ties go to the lower tag id.
    pub leader: T,
}

impl<T: Copy> Split<T> {
    /// The split, if the group has at least [`GROUP_MIN_VOTES`].
    fn of<const N: usize>(
        tags: [T; N],
        count: impl Fn(T) -> u32,
        id: fn(T) -> TagId,
    ) -> Option<Self> {
        let parts: Vec<(T, u32)> = tags.into_iter().map(|t| (t, count(t))).collect();
        let total: u32 = parts.iter().map(|(_, n)| n).sum();
        if total < GROUP_MIN_VOTES {
            return None;
        }
        let (leader, _) = *parts.iter().min_by_key(|(t, n)| (Reverse(*n), id(*t)))?;
        Some(Self {
            parts,
            total,
            leader,
        })
    }

    /// The votes for `part`.
    #[must_use]
    pub fn votes(&self, part: T) -> u32
    where
        T: PartialEq,
    {
        self.parts
            .iter()
            .find(|(t, _)| *t == part)
            .map_or(0, |(_, n)| *n)
    }
}

/// A character pair readers agree on: one side with at least two thirds of
/// the pair's votes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct CharacterLean {
    pub side: CharacterTrait,
    /// The side's votes.
    pub votes: u32,
    /// Both sides' votes together.
    pub pair_votes: u32,
}

/// Everything readers voted on a book, for the book header's "Votes"
/// popover.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct VoteBreakdown {
    /// The votes over the 14 moods.
    pub mood_votes: u32,
    /// Every mood with at least [`BREAKDOWN_MIN_MOOD_VOTES`], the most voted
    /// first, then by tag id.
    pub moods: Vec<(Mood, u32)>,
    /// See [`rarely`].
    pub rarely: Vec<Mood>,
    /// With at least [`GROUP_MIN_VOTES`] pace votes.
    pub pace: Option<Split<Pace>>,
    /// With at least [`GROUP_MIN_VOTES`] drive votes.
    pub drive: Option<Split<Drive>>,
    /// The pairs readers agree on, in [`TraitPair::ALL`]'s order.
    pub characters: Vec<CharacterLean>,
}

/// A book's votes laid out for the "Votes" popover.
#[must_use]
pub fn breakdown(votes: &BookVotes) -> VoteBreakdown {
    VoteBreakdown {
        mood_votes: votes.mood_votes(),
        moods: by_votes(votes)
            .into_iter()
            .filter(|(_, n)| *n >= BREAKDOWN_MIN_MOOD_VOTES)
            .collect(),
        rarely: rarely(votes),
        pace: Split::of(Pace::ALL, |p| votes.pace(p), Pace::id),
        drive: Split::of(Drive::ALL, |d| votes.drive(d), Drive::id),
        characters: TraitPair::ALL
            .into_iter()
            .filter_map(|pair| lean(votes, pair))
            .collect(),
    }
}

fn lean(votes: &BookVotes, pair: TraitPair) -> Option<CharacterLean> {
    let [a, b] = pair.sides();
    let (va, vb) = (votes.character(a), votes.character(b));
    let pair_votes = va + vb;
    if pair_votes < TRAIT_MIN_PAIR_VOTES {
        return None;
    }
    let (num, den) = TRAIT_MIN_SHARE;
    [(a, va), (b, vb)]
        .into_iter()
        .find(|(_, n)| n * den >= pair_votes * num)
        .map(|(side, n)| CharacterLean {
            side,
            votes: n,
            pair_votes,
        })
}
