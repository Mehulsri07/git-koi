//! Where the lilypads float: one pad per owned public repository, laid out
//! deterministically on a fixed-size pond.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use crate::history::{
    AccountSnapshot, ConflictingRepository, RepositoryObservation, RepositoryVisibility,
};

/// Width of the pond in native pixels.
pub const POND_WIDTH_PIXELS: u32 = 416;
/// Height of the pond in native pixels.
pub const POND_HEIGHT_PIXELS: u32 = 192;
/// The most pads a pond holds.
pub const MAX_PADS: usize = 12;

const PLACEMENT_ATTEMPTS: u64 = 64;
const PAD_GAP_PIXELS: f64 = 2.0;
const SATELLITE_GAP_PIXELS: f64 = 3.0;
const SATELLITE_SPREAD_PIXELS: u64 = 9;
const ANCHOR_MIN_LEVEL: u8 = 4;
const SATELLITE_MAX_LEVEL: u8 = 2;
const FLOWER_ORDER: [Bloom; 4] = [Bloom::Pink, Bloom::White, Bloom::Purple, Bloom::Bud];

/// How big a pad is, from one through five, set by the owner's commit count.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct PadSize(u8);

impl PadSize {
    /// The size for an owner's commit count: 0 to 10 is one, 11 to 20 two,
    /// 21 to 40 three, 41 to 80 four, and 81 or more five.
    pub fn from_commits(owner_commit_count: u64) -> Self {
        Self(match owner_commit_count {
            0..=10 => 1,
            11..=20 => 2,
            21..=40 => 3,
            41..=80 => 4,
            _ => 5,
        })
    }

    /// The size level, from one through five.
    pub fn level(self) -> u8 {
        self.0
    }

    /// The pad's width in native pixels.
    pub fn diameter_pixels(self) -> u32 {
        8 + 8 * u32::from(self.0)
    }

    /// The next size down, or `None` for the smallest.
    pub fn smaller(self) -> Option<Self> {
        (self.0 > 1).then(|| Self(self.0 - 1))
    }

    fn radius_pixels(self) -> f64 {
        f64::from(self.diameter_pixels()) / 2.0
    }
}

/// The flower a pad carries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Bloom {
    /// The flower of the most common language.
    Pink,
    /// The flower of the second most common language.
    White,
    /// The flower of the third most common language.
    Purple,
    /// The bud of the fourth most common language.
    Bud,
}

/// One lilypad.
#[derive(Clone, Debug, PartialEq)]
pub struct Pad {
    /// The repository this pad stands for.
    pub repository_id: u64,
    /// The pad's center in native pixels, origin at the top left.
    pub center: [f64; 2],
    /// The size after placement, which may be smaller than the commits earned.
    pub size: PadSize,
    /// The flower, if the pad is its language's biggest and the language ranks.
    pub bloom: Option<Bloom>,
    /// The angle of the pad's notch, from zero up to a full turn.
    pub notch_radians: f64,
}

/// Every pad that fits on the pond.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PondLayout {
    /// Pads ordered by repository ID.
    pub pads: Vec<Pad>,
}

/// Why no pond could be laid out. Each message names the repair.
#[derive(Debug, Eq, PartialEq)]
pub enum PondError {
    /// The snapshot predates owned-repository commit counts.
    NoOwnedRepositories,
    /// Two observations disagree about one repository.
    ConflictingRepository(ConflictingRepository),
}

impl fmt::Display for PondError {
    fn fmt(&self, output_formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoOwnedRepositories => write!(
                output_formatter,
                "the pond needs a snapshot collected by git-koi 2.0; run git-koi collect again"
            ),
            Self::ConflictingRepository(conflict) => conflict.fmt(output_formatter),
        }
    }
}

impl Error for PondError {}

struct Candidate<'a> {
    repository_id: u64,
    size: PadSize,
    language: Option<&'a str>,
    bloom: Option<Bloom>,
}

/// Lay out one pad per owned public non-fork repository, at most twelve,
/// keeping those with the most owner commits.
///
/// The layout depends only on the snapshot: the same snapshot always gives the
/// same pond. This operation performs no I/O.
pub fn layout_pond(account_snapshot: &AccountSnapshot) -> Result<PondLayout, PondError> {
    let owned_repositories = account_snapshot
        .owned_repositories
        .as_ref()
        .ok_or(PondError::NoOwnedRepositories)?;

    let mut observations: BTreeMap<u64, &RepositoryObservation> = BTreeMap::new();
    for observation in &account_snapshot.repositories {
        if let Some(previous) = observations.insert(observation.repository_id, observation)
            && previous != observation
        {
            return Err(PondError::ConflictingRepository(ConflictingRepository {
                repository_id: observation.repository_id,
            }));
        }
    }

    let mut commit_counts: BTreeMap<u64, u64> = BTreeMap::new();
    for owned_repository in owned_repositories {
        let is_eligible = observations
            .get(&owned_repository.repository_id)
            .is_some_and(|observation| {
                observation.visibility == RepositoryVisibility::Public && !observation.is_fork
            });
        if is_eligible {
            commit_counts
                .entry(owned_repository.repository_id)
                .and_modify(|count| *count = (*count).max(owned_repository.owner_commit_count))
                .or_insert(owned_repository.owner_commit_count);
        }
    }

    let mut ranked: Vec<(u64, u64)> = commit_counts.into_iter().collect();
    ranked.sort_by(|(first_id, first_count), (second_id, second_count)| {
        second_count.cmp(first_count).then(first_id.cmp(second_id))
    });
    ranked.truncate(MAX_PADS);

    let mut candidates: Vec<Candidate> = ranked
        .iter()
        .map(|(repository_id, commit_count)| Candidate {
            repository_id: *repository_id,
            size: PadSize::from_commits(*commit_count),
            language: observations[repository_id].primary_language.as_deref(),
            bloom: None,
        })
        .collect();
    assign_blooms(&mut candidates);

    candidates.sort_by(|first, second| {
        second
            .size
            .cmp(&first.size)
            .then(first.repository_id.cmp(&second.repository_id))
    });

    let mut pads: Vec<Pad> = Vec::new();
    for candidate in &candidates {
        if let Some((center, size)) = place(account_snapshot.account_id, candidate, &pads) {
            pads.push(Pad {
                repository_id: candidate.repository_id,
                center,
                size,
                bloom: candidate.bloom,
                notch_radians: (splitmix64(candidate.repository_id) % 6283) as f64 / 1000.0,
            });
        }
    }
    pads.sort_by_key(|pad| pad.repository_id);
    Ok(PondLayout { pads })
}

/// Give each of the four most common languages a flower on its biggest pad.
fn assign_blooms(candidates: &mut [Candidate]) {
    let mut pad_counts: BTreeMap<&str, usize> = BTreeMap::new();
    let mut biggest: BTreeMap<&str, (PadSize, u64)> = BTreeMap::new();
    for candidate in candidates.iter() {
        let Some(language) = candidate.language else {
            continue;
        };
        *pad_counts.entry(language).or_insert(0) += 1;
        let entry = biggest
            .entry(language)
            .or_insert((candidate.size, candidate.repository_id));
        if candidate.size > entry.0
            || (candidate.size == entry.0 && candidate.repository_id < entry.1)
        {
            *entry = (candidate.size, candidate.repository_id);
        }
    }
    // BTreeMap iterates by name, and the stable sort keeps that order on ties.
    let mut languages: Vec<(&str, usize)> = pad_counts.into_iter().collect();
    languages.sort_by_key(|language| std::cmp::Reverse(language.1));
    let winners: Vec<(u64, Bloom)> = languages
        .iter()
        .zip(FLOWER_ORDER)
        .map(|((language, _), bloom)| (biggest[language].1, bloom))
        .collect();
    for (repository_id, bloom) in winners {
        if let Some(candidate) = candidates
            .iter_mut()
            .find(|candidate| candidate.repository_id == repository_id)
        {
            candidate.bloom = Some(bloom);
        }
    }
}

fn place(account_id: u64, candidate: &Candidate, placed: &[Pad]) -> Option<([f64; 2], PadSize)> {
    let mut size = candidate.size;
    loop {
        let radius = size.radius_pixels();
        for attempt in 0..PLACEMENT_ATTEMPTS {
            let hash = splitmix64(account_id ^ candidate.repository_id.rotate_left(17) ^ attempt);
            let anchor = placed
                .iter()
                .filter(|pad| pad.size.level() >= ANCHOR_MIN_LEVEL)
                .min_by_key(|pad| pad.repository_id);
            let center = match anchor {
                Some(anchor) if size.level() <= SATELLITE_MAX_LEVEL && attempt % 2 == 0 => {
                    let angle = ((hash >> 20) % 6283) as f64 / 1000.0;
                    let distance = anchor.size.radius_pixels()
                        + radius
                        + SATELLITE_GAP_PIXELS
                        + (hash % SATELLITE_SPREAD_PIXELS) as f64;
                    [
                        anchor.center[0] + distance * angle.cos(),
                        anchor.center[1] + distance * angle.sin(),
                    ]
                }
                _ => {
                    let width = u64::from(POND_WIDTH_PIXELS) - 2 * radius as u64;
                    let height = u64::from(POND_HEIGHT_PIXELS) - 2 * radius as u64;
                    [
                        radius + (hash % width) as f64,
                        radius + ((hash >> 32) % height) as f64,
                    ]
                }
            };
            if fits(center, radius, placed) {
                return Some((center, size));
            }
        }
        size = size.smaller()?;
    }
}

fn fits(center: [f64; 2], radius: f64, placed: &[Pad]) -> bool {
    let inside = center[0] - radius >= 0.0
        && center[0] + radius <= f64::from(POND_WIDTH_PIXELS)
        && center[1] - radius >= 0.0
        && center[1] + radius <= f64::from(POND_HEIGHT_PIXELS);
    inside
        && placed.iter().all(|pad| {
            let distance = (center[0] - pad.center[0]).hypot(center[1] - pad.center[1]);
            distance >= radius + pad.size.radius_pixels() + PAD_GAP_PIXELS
        })
}

fn splitmix64(seed: u64) -> u64 {
    let mut mixed = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    mixed ^ (mixed >> 31)
}
