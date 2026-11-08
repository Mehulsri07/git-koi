//! The day's movement, chosen from recent activity by one ordered set of rules.

use crate::koi::{KoiDevelopment, RecentActivity};

/// One movement per run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Behavior {
    /// Sits low in the water, blue-tinted, barely moving.
    DeepRest,
    /// Hovers in place a little deeper, with a slow beat and glide.
    Rest,
    /// Hovers at the surface and gulps.
    SurfaceFeeding,
    /// Two fast laps of strong bursts and glides.
    Dart,
    /// One slow lap; the everyday swim.
    Cruise,
}

/// Why the koi moves as it does: the first rule that matched, with its evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BehaviorReason {
    /// The observation date ends at least the deep-rest threshold of quiet days.
    DeepRest {
        /// Consecutive quiet days ending on the observation date.
        quiet_days: u32,
    },
    /// The observation date ends at least the rest threshold of quiet days.
    Rest {
        /// Consecutive quiet days ending on the observation date.
        quiet_days: u32,
    },
    /// The observation date is the first day with contributions in its week.
    FirstContributionOfWeek,
    /// The total crossed a milestone within the last seven days.
    CrossedMilestone {
        /// The milestone crossed; the largest when several were.
        milestone: u64,
    },
    /// The observation date finishes a busy week.
    BusyWeek {
        /// Contributions in the finished week.
        week_contributions: u64,
    },
    /// No other rule matched.
    Steady,
}

const DEEP_REST_QUIET_DAYS: u32 = 60;
const REST_QUIET_DAYS: u32 = 14;
/// Totals whose crossing earns a week of darting.
const DART_MILESTONES: [u64; 6] = [1_000, 5_000, 10_000, 25_000, 50_000, 100_000];
/// A busy week is compared with at most this many of the latest complete weeks.
const DART_COMPARED_WEEKS: usize = 52;
/// A top tenth means little with fewer complete weeks than this.
const DART_MIN_COMPLETE_WEEKS: usize = 10;
/// A busy week is matched or beaten by at most one in this many compared weeks.
const DART_WEEK_SHARE_DENOMINATOR: usize = 10;
const DAYS_PER_WEEK: usize = 7;

/// Choose the movement for the day after the observation date. A nightly run judges
/// the day that just ended, and the koi shows the result all of the next day. The
/// first matching rule wins:
///
/// | Behavior | When the observation date |
/// | --- | --- |
/// | Deep rest | ends DEEP_REST_QUIET_DAYS or more quiet days |
/// | Rest | ends REST_QUIET_DAYS or more quiet days |
/// | Surface feeding | is the first day with contributions in its Monday-Sunday week |
/// | Dart | falls within seven days of the total crossing a milestone |
/// | Dart | is a Sunday that finishes a busy week |
/// | Cruise | is anything else |
///
/// A finished week is busy when at most one in ten of the latest
/// DART_COMPARED_WEEKS complete weeks before it match or beat its total, and at
/// least DART_MIN_COMPLETE_WEEKS weeks are compared. So a steady account cruises,
/// and the koi darts at most one day a week beyond milestones. This reads no clock
/// and changes no growth, colors, or marks.
pub fn select_behavior(koi_development: &KoiDevelopment) -> Behavior {
    explain_behavior(koi_development).0
}

/// Choose the movement as `select_behavior` does, and say which rule chose it. The
/// two share this one rule path, so `select_behavior(d)` is `explain_behavior(d).0`.
pub fn explain_behavior(koi_development: &KoiDevelopment) -> (Behavior, BehaviorReason) {
    let recent_activity = &koi_development.recent_activity;
    let quiet_days = recent_activity.quiet_day_count;
    if quiet_days >= DEEP_REST_QUIET_DAYS {
        (Behavior::DeepRest, BehaviorReason::DeepRest { quiet_days })
    } else if quiet_days >= REST_QUIET_DAYS {
        (Behavior::Rest, BehaviorReason::Rest { quiet_days })
    } else if is_first_contribution_of_week(recent_activity) {
        (
            Behavior::SurfaceFeeding,
            BehaviorReason::FirstContributionOfWeek,
        )
    } else if let Some(milestone) =
        crossed_milestone(koi_development.total_contributions, recent_activity)
    {
        (
            Behavior::Dart,
            BehaviorReason::CrossedMilestone { milestone },
        )
    } else if let Some(week_contributions) = finished_busy_week_total(recent_activity) {
        (
            Behavior::Dart,
            BehaviorReason::BusyWeek { week_contributions },
        )
    } else {
        (Behavior::Cruise, BehaviorReason::Steady)
    }
}

fn is_first_contribution_of_week(recent_activity: &RecentActivity) -> bool {
    matches!(
        recent_activity.observation_week_contributions.split_last(),
        Some((&observed_day, earlier_days))
            if observed_day > 0 && earlier_days.iter().all(|&day_contributions| day_contributions == 0)
    )
}

fn crossed_milestone(total_contributions: u64, recent_activity: &RecentActivity) -> Option<u64> {
    let earlier_total = total_contributions.saturating_sub(recent_activity.recent_contributions);
    DART_MILESTONES
        .iter()
        .rev()
        .copied()
        .find(|&milestone| earlier_total < milestone && milestone <= total_contributions)
}

fn finished_busy_week_total(recent_activity: &RecentActivity) -> Option<u64> {
    let finished_week = &recent_activity.observation_week_contributions;
    let complete_weeks = &recent_activity.complete_week_contributions;
    let compared_weeks =
        &complete_weeks[complete_weeks.len().saturating_sub(DART_COMPARED_WEEKS)..];
    let week_total: u64 = finished_week.iter().sum();
    let matching_week_count = compared_weeks
        .iter()
        .filter(|&&week_contributions| week_contributions >= week_total)
        .count();
    let is_busy = finished_week.len() == DAYS_PER_WEEK
        && compared_weeks.len() >= DART_MIN_COMPLETE_WEEKS
        && matching_week_count * DART_WEEK_SHARE_DENOMINATOR <= compared_weeks.len();
    is_busy.then_some(week_total)
}
