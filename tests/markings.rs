use chrono::{Datelike, Days, NaiveDate};
use git_koi::history::{AccountSnapshot, ContributionDay, ContributionHistory, ContributionWeek};
use git_koi::koi::{KoiDevelopment, RecentStreak, derive_koi_development};
use git_koi::markings::calculate_streak_marks;

#[test]
fn streak_fins_build_linearly_to_full_at_thirteen_weeks() {
    for (streak_week_count, expected_strength) in
        [(1, 1.0 / 13.0), (6, 6.0 / 13.0), (13, 1.0), (20, 1.0)]
    {
        let koi_development = develop_with_streaks(
            streak_week_count,
            Some(RecentStreak::Current(build_weeks(streak_week_count))),
        );
        assert_close(
            calculate_streak_marks(&koi_development).streak_fin_strength,
            expected_strength,
        );
    }
}

#[test]
fn streak_fins_fade_to_nothing_six_and_a_half_weeks_after_a_streak_ends() {
    let strength_after = |days_since_end| {
        calculate_streak_marks(&develop_after_streak_end(13, days_since_end)).streak_fin_strength
    };
    assert_close(strength_after(0), 1.0);
    // Six and a half weeks is 45.5 days: still showing on day 45, gone from day 46.
    let mut previous_strength = strength_after(0);
    for days_since_end in 1..=45 {
        let fading_strength = strength_after(days_since_end);
        assert!(
            0.0 < fading_strength && fading_strength < previous_strength,
            "{days_since_end} days after: {fading_strength}"
        );
        previous_strength = fading_strength;
    }
    assert_close(strength_after(46), 0.0);
    assert_close(strength_after(365), 0.0);
}

#[test]
fn a_short_streak_fades_from_the_strength_it_reached() {
    let strength_of = |streak_week_count, days_since_end| {
        calculate_streak_marks(&develop_after_streak_end(streak_week_count, days_since_end))
            .streak_fin_strength
    };
    assert_close(strength_of(4, 0), 4.0 / 13.0);
    for days_since_end in [7, 28] {
        assert_close(
            strength_of(4, days_since_end),
            4.0 / 13.0 * strength_of(13, days_since_end),
        );
    }
}

#[test]
fn the_longest_streak_brings_colors_from_dull_to_vivid_then_glimmer() {
    let marks_at = |longest_week_count| {
        calculate_streak_marks(&develop_with_streaks(longest_week_count, None))
    };
    assert_close(marks_at(0).color_vibrancy, 0.0);
    assert!(marks_at(1).color_vibrancy > 0.4 && marks_at(1).color_vibrancy < 0.5);
    assert_close(marks_at(4).color_vibrancy, 1.0);
    assert_close(marks_at(52).color_vibrancy, 2.0);
    assert_close(marks_at(52).glimmer_strength, 0.0);
    assert!(marks_at(104).glimmer_strength > 0.0 && marks_at(104).glimmer_strength < 1.0);
    assert_close(marks_at(208).glimmer_strength, 1.0);
    assert_eq!(marks_at(400), marks_at(208));
    let mut previous_marks = marks_at(0);
    for longest_week_count in 1..=260 {
        let longest_marks = marks_at(longest_week_count);
        assert!(longest_marks.color_vibrancy >= previous_marks.color_vibrancy);
        assert!(longest_marks.glimmer_strength >= previous_marks.glimmer_strength);
        previous_marks = longest_marks;
    }
}

#[test]
fn quiet_weeks_fade_the_fins_without_lowering_the_longest_streak_marks() {
    let first_monday: NaiveDate = "2025-09-01".parse().unwrap();
    let mut daily_counts = vec![1; 13 * 7];
    let active_marks =
        calculate_streak_marks(&develop_from_daily_counts(first_monday, &daily_counts));
    assert_close(active_marks.streak_fin_strength, 1.0);
    let mut previous_marks = active_marks;
    for _ in 0..10 {
        daily_counts.extend([0; 7]);
        let quiet_marks =
            calculate_streak_marks(&develop_from_daily_counts(first_monday, &daily_counts));
        assert!(quiet_marks.streak_fin_strength <= previous_marks.streak_fin_strength);
        assert_eq!(quiet_marks.color_vibrancy, active_marks.color_vibrancy);
        assert_eq!(quiet_marks.glimmer_strength, active_marks.glimmer_strength);
        previous_marks = quiet_marks;
    }
    assert_eq!(previous_marks.streak_fin_strength, 0.0);
}

fn develop_with_streaks(
    longest_week_count: usize,
    recent_streak: Option<RecentStreak>,
) -> KoiDevelopment {
    let mut koi_development = read_fixture_development();
    koi_development.streaks.longest = build_weeks(longest_week_count);
    koi_development.streaks.recent = recent_streak;
    koi_development
}

fn develop_after_streak_end(streak_week_count: usize, days_since_end: u64) -> KoiDevelopment {
    let mut koi_development = read_fixture_development();
    let ended_on = koi_development.observed_on - Days::new(days_since_end);
    koi_development.streaks.longest = build_weeks(streak_week_count);
    koi_development.streaks.recent = Some(RecentStreak::Ended {
        weeks: build_weeks(streak_week_count),
        ended_on,
    });
    koi_development
}

fn develop_from_daily_counts(first_monday: NaiveDate, daily_counts: &[u32]) -> KoiDevelopment {
    let mut account_snapshot: AccountSnapshot =
        serde_json::from_str(include_str!("fixtures/account-with-history.json")).unwrap();
    let contribution_days: Vec<_> = daily_counts
        .iter()
        .enumerate()
        .map(|(day_offset, &contribution_count)| ContributionDay {
            date: first_monday + Days::new(day_offset as u64),
            contribution_count,
        })
        .collect();
    account_snapshot.contribution_history = Some(ContributionHistory {
        observed_on: contribution_days.last().unwrap().date,
        days: contribution_days,
    });
    derive_koi_development(&account_snapshot).unwrap()
}

fn read_fixture_development() -> KoiDevelopment {
    let account_snapshot =
        serde_json::from_str(include_str!("fixtures/account-with-history.json")).unwrap();
    derive_koi_development(&account_snapshot).unwrap()
}

/// Fully active weeks; marks depend only on how many there are.
fn build_weeks(week_count: usize) -> Vec<ContributionWeek> {
    let first_monday: NaiveDate = "2024-01-01".parse().unwrap();
    (0..week_count)
        .map(|week_index| ContributionWeek {
            iso_week: (first_monday + Days::new(7 * week_index as u64)).iso_week(),
            active_day_count: 7,
            observed_day_count: 7,
        })
        .collect()
}

fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1e-12,
        "{actual} is not {expected}"
    );
}
