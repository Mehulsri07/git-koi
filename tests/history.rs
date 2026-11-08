use std::collections::BTreeMap;

use chrono::{Datelike, Days, NaiveDate};
use git_koi::history::{
    AccountSnapshot, ConflictingRepository, ContributionDay, ContributionHistory,
    ContributionHistoryError, LanguageColor, OwnedRepository, RepositoryVisibility,
    calculate_language_shares, summarize_contributions,
};

const ACCOUNT_JSON: &str = include_str!("fixtures/account.json");
const ACCOUNT_WITH_HISTORY_JSON: &str = include_str!("fixtures/account-with-history.json");

#[test]
fn language_shares_count_only_eligible_projects() {
    let account_snapshot: AccountSnapshot = serde_json::from_str(ACCOUNT_JSON).unwrap();

    let language_shares = calculate_language_shares(&account_snapshot).unwrap();

    assert_eq!(
        language_shares,
        BTreeMap::from([
            ("C++".to_owned(), 0.2),
            ("Python".to_owned(), 0.2),
            ("Rust".to_owned(), 0.6),
        ])
    );
}

#[test]
fn language_shares_stay_identical_after_reordering_and_duplicate_observations() {
    let mut account_snapshot: AccountSnapshot = serde_json::from_str(ACCOUNT_JSON).unwrap();
    let original_shares = calculate_language_shares(&account_snapshot).unwrap();
    account_snapshot
        .repositories
        .push(account_snapshot.repositories[0].clone());
    account_snapshot.repositories.reverse();

    for _ in 0..account_snapshot.repositories.len() {
        account_snapshot.repositories.rotate_left(1);
        assert_eq!(
            calculate_language_shares(&account_snapshot).unwrap(),
            original_shares
        );
    }
}

#[test]
fn missing_observations_cannot_be_read_as_a_confirmed_empty_collection() {
    let mut snapshot_document: serde_json::Value = serde_json::from_str(ACCOUNT_JSON).unwrap();
    snapshot_document["repositories"] = serde_json::json!([]);
    let empty_snapshot: AccountSnapshot =
        serde_json::from_value(snapshot_document.clone()).unwrap();
    assert!(
        calculate_language_shares(&empty_snapshot)
            .unwrap()
            .is_empty()
    );

    snapshot_document
        .as_object_mut()
        .unwrap()
        .remove("repositories");
    assert!(serde_json::from_value::<AccountSnapshot>(snapshot_document).is_err());
}

#[test]
fn missing_language_cannot_be_read_as_a_known_absence() {
    let mut snapshot_document: serde_json::Value = serde_json::from_str(ACCOUNT_JSON).unwrap();
    snapshot_document["repositories"][0]["primary_language"] = serde_json::Value::Null;
    let null_language_snapshot: AccountSnapshot =
        serde_json::from_value(snapshot_document.clone()).unwrap();
    assert_eq!(
        calculate_language_shares(&null_language_snapshot).unwrap()["Rust"],
        0.5
    );

    snapshot_document["repositories"][0]
        .as_object_mut()
        .unwrap()
        .remove("primary_language");
    assert!(serde_json::from_value::<AccountSnapshot>(snapshot_document).is_err());
}

#[test]
fn conflicting_observations_fail_regardless_of_order_or_eligibility() {
    let account_snapshot: AccountSnapshot = serde_json::from_str(ACCOUNT_JSON).unwrap();
    let mut repository_with_conflicting_language = account_snapshot.repositories[0].clone();
    repository_with_conflicting_language.primary_language = Some("Python".to_owned());
    let mut repository_with_conflicting_visibility = account_snapshot.repositories[0].clone();
    repository_with_conflicting_visibility.visibility = RepositoryVisibility::Private;

    for conflicting_observation in [
        repository_with_conflicting_language,
        repository_with_conflicting_visibility,
    ] {
        let expected_error = ConflictingRepository {
            repository_id: conflicting_observation.repository_id,
        };
        let mut conflicting_snapshot = account_snapshot.clone();
        conflicting_snapshot
            .repositories
            .push(conflicting_observation);
        assert_eq!(
            calculate_language_shares(&conflicting_snapshot),
            Err(expected_error)
        );
        conflicting_snapshot.repositories.reverse();
        assert!(calculate_language_shares(&conflicting_snapshot).is_err());
    }
}

#[test]
fn snapshot_writer_and_reader_preserve_observations_and_language_shares() {
    let account_snapshot: AccountSnapshot = serde_json::from_str(ACCOUNT_JSON).unwrap();
    let snapshot_bytes = serde_json::to_vec(&account_snapshot).unwrap();
    let restored_snapshot: AccountSnapshot = serde_json::from_slice(&snapshot_bytes).unwrap();

    assert_eq!(restored_snapshot, account_snapshot);
    assert_eq!(
        calculate_language_shares(&restored_snapshot).unwrap(),
        calculate_language_shares(&account_snapshot).unwrap()
    );
}

#[test]
fn incompatible_documents_cannot_enter_language_calculation() {
    let snapshot_document: serde_json::Value = serde_json::from_str(ACCOUNT_JSON).unwrap();
    let mut snapshot_with_unsupported_version = snapshot_document.clone();
    snapshot_with_unsupported_version["schema_version"] = serde_json::json!(999);
    let mut snapshot_without_version = snapshot_document.clone();
    snapshot_without_version
        .as_object_mut()
        .unwrap()
        .remove("schema_version");
    let mut snapshot_with_unknown_account_field = snapshot_document.clone();
    snapshot_with_unknown_account_field["unexpected_field"] = serde_json::json!(true);
    let mut snapshot_with_unknown_repository_field = snapshot_document;
    snapshot_with_unknown_repository_field["repositories"][0]["unexpected_field"] =
        serde_json::json!(true);

    for incompatible_document in [
        snapshot_with_unsupported_version,
        snapshot_without_version,
        snapshot_with_unknown_account_field,
        snapshot_with_unknown_repository_field,
    ] {
        assert!(serde_json::from_value::<AccountSnapshot>(incompatible_document).is_err());
    }
}

#[test]
fn weekly_summary_preserves_inactive_and_partial_weeks_across_a_year_boundary() {
    let account_snapshot: AccountSnapshot =
        serde_json::from_str(ACCOUNT_WITH_HISTORY_JSON).unwrap();
    let contribution_summary = summarize_contributions(&account_snapshot).unwrap();
    let weekly_participation: Vec<_> = contribution_summary
        .weeks
        .iter()
        .map(|contribution_week| {
            (
                contribution_week.iso_week.year(),
                contribution_week.iso_week.week(),
                contribution_week.active_day_count,
                contribution_week.observed_day_count,
            )
        })
        .collect();

    assert_eq!(contribution_summary.total_contributions, 14);
    assert_eq!(
        weekly_participation,
        [
            (2025, 51, 1, 2),
            (2025, 52, 3, 7),
            (2026, 1, 0, 7),
            (2026, 2, 2, 2)
        ]
    );
    assert_eq!(
        contribution_summary.weeks.last().unwrap().iso_week,
        contribution_summary.observed_on.iso_week()
    );
}

#[test]
fn equal_totals_preserve_different_weekday_and_weekend_participation() {
    let first_monday: NaiveDate = "2026-01-05".parse().unwrap();
    let weekday_counts = [1, 1, 1, 1, 1, 0, 0];
    let saturday_counts = [0, 0, 0, 0, 0, 5, 0];
    for (weekly_counts, expected_active_days) in [(weekday_counts, 5), (saturday_counts, 1)] {
        let contribution_days: Vec<_> = (0..12 * 7)
            .zip(weekly_counts.into_iter().cycle())
            .map(|(day_offset, contribution_count)| ContributionDay {
                date: first_monday
                    .checked_add_days(Days::new(day_offset))
                    .unwrap(),
                contribution_count,
            })
            .collect();
        let mut account_snapshot: AccountSnapshot = serde_json::from_str(ACCOUNT_JSON).unwrap();
        account_snapshot.contribution_history = Some(ContributionHistory {
            observed_on: contribution_days.last().unwrap().date,
            days: contribution_days,
        });
        let contribution_summary = summarize_contributions(&account_snapshot).unwrap();
        assert_eq!(contribution_summary.total_contributions, 60);
        assert_eq!(
            contribution_summary
                .weeks
                .iter()
                .map(|contribution_week| contribution_week.active_day_count)
                .collect::<Vec<_>>(),
            vec![expected_active_days; 12]
        );
    }
}

#[test]
fn weekly_measurements_ignore_observation_order_and_identical_duplicates() {
    let mut account_snapshot: AccountSnapshot =
        serde_json::from_str(ACCOUNT_WITH_HISTORY_JSON).unwrap();
    let original_summary = summarize_contributions(&account_snapshot).unwrap();
    let contribution_days = &mut account_snapshot.contribution_history.as_mut().unwrap().days;
    contribution_days.push(contribution_days[0].clone());
    contribution_days.reverse();
    assert_eq!(
        summarize_contributions(&account_snapshot).unwrap(),
        original_summary
    );
}

#[test]
fn supported_snapshot_versions_preserve_observations_and_language_shares() {
    let mut legacy_snapshot: AccountSnapshot = serde_json::from_str(ACCOUNT_JSON).unwrap();
    assert_eq!(
        serde_json::to_value(&legacy_snapshot).unwrap(),
        serde_json::from_str::<serde_json::Value>(ACCOUNT_JSON).unwrap()
    );
    let original_shares = calculate_language_shares(&legacy_snapshot).unwrap();
    let dated_snapshot: AccountSnapshot = serde_json::from_str(ACCOUNT_WITH_HISTORY_JSON).unwrap();
    legacy_snapshot.contribution_history = dated_snapshot.contribution_history;
    let snapshot_bytes = serde_json::to_vec(&legacy_snapshot).unwrap();
    let restored_snapshot: AccountSnapshot = serde_json::from_slice(&snapshot_bytes).unwrap();
    assert_eq!(restored_snapshot, legacy_snapshot);
    assert_eq!(
        calculate_language_shares(&restored_snapshot).unwrap(),
        original_shares
    );
    assert_eq!(
        summarize_contributions(&restored_snapshot).unwrap(),
        summarize_contributions(&legacy_snapshot).unwrap()
    );
}

#[test]
fn colored_snapshots_round_trip_with_absent_colors_kept_distinct() {
    let mut account_snapshot: AccountSnapshot =
        serde_json::from_str(ACCOUNT_WITH_HISTORY_JSON).unwrap();
    assert_eq!(
        serde_json::to_value(&account_snapshot).unwrap(),
        serde_json::from_str::<serde_json::Value>(ACCOUNT_WITH_HISTORY_JSON).unwrap()
    );
    account_snapshot
        .language_colors
        .as_mut()
        .unwrap()
        .insert("Text".to_owned(), None);
    let snapshot_bytes = serde_json::to_vec(&account_snapshot).unwrap();
    let restored_snapshot: AccountSnapshot = serde_json::from_slice(&snapshot_bytes).unwrap();
    assert_eq!(restored_snapshot, account_snapshot);
    assert_eq!(
        restored_snapshot.language_colors.unwrap()["Rust"],
        Some(LanguageColor([0xde, 0xa5, 0x84]))
    );
}

#[test]
fn unavailable_history_differs_from_confirmed_empty_history() {
    let mut account_snapshot: AccountSnapshot = serde_json::from_str(ACCOUNT_JSON).unwrap();
    assert_eq!(
        summarize_contributions(&account_snapshot),
        Err(ContributionHistoryError::MissingHistory)
    );
    account_snapshot.contribution_history = Some(ContributionHistory {
        observed_on: "2026-01-06".parse().unwrap(),
        days: Vec::new(),
    });
    let empty_summary = summarize_contributions(&account_snapshot).unwrap();
    assert_eq!(empty_summary.total_contributions, 0);
    assert!(empty_summary.weeks.is_empty());
}

#[test]
fn missing_dates_cannot_silently_shorten_weekly_participation() {
    let account_snapshot: AccountSnapshot =
        serde_json::from_str(ACCOUNT_WITH_HISTORY_JSON).unwrap();
    let last_day_index = account_snapshot
        .contribution_history
        .as_ref()
        .unwrap()
        .days
        .len()
        - 1;
    for removed_index in [1, last_day_index] {
        let mut corrupted_snapshot = account_snapshot.clone();
        let contribution_history = corrupted_snapshot.contribution_history.as_mut().unwrap();
        let removed_day = contribution_history.days.remove(removed_index);
        assert_eq!(
            summarize_contributions(&corrupted_snapshot),
            Err(ContributionHistoryError::MissingDate(removed_day.date))
        );
    }
}

#[test]
fn conflicting_counts_cannot_replace_an_observation() {
    let mut account_snapshot: AccountSnapshot =
        serde_json::from_str(ACCOUNT_WITH_HISTORY_JSON).unwrap();
    let contribution_days = &mut account_snapshot.contribution_history.as_mut().unwrap().days;
    let mut conflicting_day = contribution_days[0].clone();
    conflicting_day.contribution_count += 1;
    let expected_error = ContributionHistoryError::ConflictingDate(conflicting_day.date);
    contribution_days.push(conflicting_day);
    assert_eq!(
        summarize_contributions(&account_snapshot),
        Err(expected_error)
    );
}

#[test]
fn observations_after_the_cutoff_cannot_inflate_the_total() {
    let mut account_snapshot: AccountSnapshot =
        serde_json::from_str(ACCOUNT_WITH_HISTORY_JSON).unwrap();
    let contribution_history = account_snapshot.contribution_history.as_mut().unwrap();
    contribution_history.observed_on = contribution_history.observed_on.pred_opt().unwrap();
    let expected_error = ContributionHistoryError::AfterObservationDate {
        contribution_date: contribution_history.days.last().unwrap().date,
        observed_on: contribution_history.observed_on,
    };
    assert_eq!(
        summarize_contributions(&account_snapshot),
        Err(expected_error)
    );
}

#[test]
fn snapshot_versions_enforce_their_own_history_fields() {
    let snapshot_document: serde_json::Value =
        serde_json::from_str(ACCOUNT_WITH_HISTORY_JSON).unwrap();
    let mut legacy_with_history = snapshot_document.clone();
    legacy_with_history["schema_version"] = serde_json::json!(1);
    let mut dated_without_history = snapshot_document.clone();
    dated_without_history
        .as_object_mut()
        .unwrap()
        .remove("contribution_history");
    let mut dated_with_null_history = snapshot_document.clone();
    dated_with_null_history["contribution_history"] = serde_json::Value::Null;
    let mut history_with_unknown_field = snapshot_document.clone();
    history_with_unknown_field["contribution_history"]["unexpected_field"] =
        serde_json::json!(true);
    let mut dated_with_colors = snapshot_document.clone();
    dated_with_colors["schema_version"] = serde_json::json!(2);
    let mut colored_without_colors = snapshot_document.clone();
    colored_without_colors
        .as_object_mut()
        .unwrap()
        .remove("language_colors");
    let mut colored_with_null_colors = snapshot_document.clone();
    colored_with_null_colors["language_colors"] = serde_json::Value::Null;
    let mut malformed_color = snapshot_document;
    malformed_color["language_colors"]["Rust"] = serde_json::json!("dea584");
    for incompatible_document in [
        legacy_with_history,
        dated_without_history,
        dated_with_null_history,
        history_with_unknown_field,
        dated_with_colors,
        colored_without_colors,
        colored_with_null_colors,
        malformed_color,
    ] {
        assert!(serde_json::from_value::<AccountSnapshot>(incompatible_document).is_err());
    }
}

#[test]
fn version_4_round_trips_owned_repositories() {
    let mut account_snapshot: AccountSnapshot =
        serde_json::from_str(ACCOUNT_WITH_HISTORY_JSON).unwrap();
    account_snapshot.owned_repositories = Some(vec![
        OwnedRepository {
            repository_id: 7,
            owner_commit_count: 120,
            name_with_owner: "Mehulsri07/StrataField".to_owned(),
        },
        OwnedRepository {
            repository_id: 9,
            owner_commit_count: 0,
            name_with_owner: "Mehulsri07/git-koi".to_owned(),
        },
    ]);
    let snapshot_document = serde_json::to_value(&account_snapshot).unwrap();
    assert_eq!(snapshot_document["schema_version"], 4);
    let restored_snapshot: AccountSnapshot = serde_json::from_value(snapshot_document).unwrap();
    assert_eq!(restored_snapshot, account_snapshot);
}

#[test]
fn version_4_needs_names_for_owned_repositories() {
    let mut account_snapshot: AccountSnapshot =
        serde_json::from_str(ACCOUNT_WITH_HISTORY_JSON).unwrap();
    account_snapshot.owned_repositories = Some(vec![OwnedRepository {
        repository_id: 7,
        owner_commit_count: 120,
        name_with_owner: "Mehulsri07/StrataField".to_owned(),
    }]);
    let mut snapshot_document = serde_json::to_value(&account_snapshot).unwrap();
    snapshot_document["owned_repositories"][0]
        .as_object_mut()
        .unwrap()
        .remove("name_with_owner");
    assert!(serde_json::from_value::<AccountSnapshot>(snapshot_document).is_err());
}

#[test]
fn version_3_loads_without_owned_repositories() {
    let account_snapshot: AccountSnapshot =
        serde_json::from_str(ACCOUNT_WITH_HISTORY_JSON).unwrap();
    assert_eq!(account_snapshot.owned_repositories, None);
}

#[test]
fn owned_repositories_is_not_a_field_before_version_4() {
    let mut snapshot_document: serde_json::Value =
        serde_json::from_str(ACCOUNT_WITH_HISTORY_JSON).unwrap();
    snapshot_document["owned_repositories"] = serde_json::json!([]);
    let error = serde_json::from_value::<AccountSnapshot>(snapshot_document).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("owned_repositories is not a field before snapshot version 4"),
        "{error}"
    );
}

#[test]
fn version_4_needs_owned_repositories() {
    let mut snapshot_document: serde_json::Value =
        serde_json::from_str(ACCOUNT_WITH_HISTORY_JSON).unwrap();
    snapshot_document["schema_version"] = serde_json::json!(4);
    let error = serde_json::from_value::<AccountSnapshot>(snapshot_document).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("missing owned_repositories in snapshot version 4"),
        "{error}"
    );
}
