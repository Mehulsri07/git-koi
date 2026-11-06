use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs;
use std::process::Command;

use chrono::{Datelike, Days, NaiveDate, Weekday};
use git_koi::coat::KoiBody;
use git_koi::collect::{CollectionError, GraphqlClient, collect_account_snapshot};
use git_koi::history::{
    ContributionHistoryError, LanguageColor, OwnedRepository, calculate_language_shares,
    summarize_contributions,
};
use git_koi::koi::derive_koi_development;
use git_koi::render::render_koi;
use serde_json::{Value, json};

const LOGIN: &str = "synthetic-koi";

#[test]
fn collected_snapshot_covers_every_page_and_day_and_renders() {
    let github = RecordedGitHub::new();
    let account_snapshot = collect_account_snapshot(&github, LOGIN, date("2026-01-03")).unwrap();

    assert_eq!(account_snapshot.account_id, 4242);
    assert_eq!(account_snapshot.repositories.len(), 4);
    assert_eq!(
        calculate_language_shares(&account_snapshot).unwrap(),
        BTreeMap::from([("Python".to_owned(), 0.5), ("Rust".to_owned(), 0.5)])
    );
    assert_eq!(
        account_snapshot.language_colors,
        Some(BTreeMap::from([
            ("Python".to_owned(), Some(LanguageColor([0x35, 0x72, 0xa5]))),
            ("Rust".to_owned(), Some(LanguageColor([0xde, 0xa5, 0x84]))),
        ]))
    );
    let contribution_history = account_snapshot.contribution_history.as_ref().unwrap();
    assert_eq!(contribution_history.days[0].date, date("2024-11-20"));
    assert_eq!(contribution_history.observed_on, date("2026-01-02"));
    let contribution_summary = summarize_contributions(&account_snapshot).unwrap();
    let expected_total: u64 = date("2024-11-20")
        .iter_days()
        .take_while(|day| *day <= date("2026-01-02"))
        .map(|day| u64::from(synthetic_count(day)))
        .sum();
    assert_eq!(contribution_summary.total_contributions, expected_total);

    let koi_frame = render_koi(
        &derive_koi_development(&account_snapshot).unwrap(),
        KoiBody::default(),
    );
    assert!(
        koi_frame
            .as_rgba_bytes()
            .chunks(4)
            .any(|pixel| pixel[3] != 0)
    );
}

#[test]
fn collection_records_owner_commits_for_owned_public_repositories() {
    let mut github = RecordedGitHub::new();
    let mut fork = owned_node(30, 5);
    fork["isFork"] = json!(true);
    let mut private = owned_node(40, 9);
    private["visibility"] = json!("PRIVATE");
    github.owned_pages = vec![repository_page(
        vec![owned_node(10, 7), fork, private, owned_node(20, 95)],
        None,
    )];
    let account_snapshot = collect_account_snapshot(&github, LOGIN, date("2026-01-03")).unwrap();
    assert_eq!(
        account_snapshot.owned_repositories,
        Some(vec![
            OwnedRepository {
                repository_id: 10,
                owner_commit_count: 7
            },
            OwnedRepository {
                repository_id: 20,
                owner_commit_count: 95
            },
        ])
    );
}

#[test]
fn an_owned_repository_without_a_default_branch_has_no_commits() {
    let mut github = RecordedGitHub::new();
    let mut empty = owned_node(10, 0);
    empty["defaultBranchRef"] = Value::Null;
    github.owned_pages = vec![repository_page(vec![empty], None)];
    let account_snapshot = collect_account_snapshot(&github, LOGIN, date("2026-01-03")).unwrap();
    assert_eq!(
        account_snapshot.owned_repositories,
        Some(vec![OwnedRepository {
            repository_id: 10,
            owner_commit_count: 0
        }])
    );
}

#[test]
fn contributed_repositories_are_not_owned() {
    let github = RecordedGitHub::new();
    let account_snapshot = collect_account_snapshot(&github, LOGIN, date("2026-01-03")).unwrap();
    let owned = account_snapshot.owned_repositories.unwrap();
    assert!(owned.iter().all(|repository| repository.repository_id != 4));
    assert_eq!(owned.len(), 2);
}

#[test]
fn the_owner_commit_query_filters_by_the_users_node_id() {
    let github = RecordedGitHub::new();
    collect_account_snapshot(&github, LOGIN, date("2026-01-03")).unwrap();
    let queries = github.queries.borrow();
    let owned_requests: Vec<_> = queries
        .iter()
        .filter(|(query, _)| query.contains("repositories(") && !query.contains("ContributedTo"))
        .collect();
    assert!(!owned_requests.is_empty());
    for (query, variables) in owned_requests {
        assert_eq!(variables["userId"], "U_synthetic");
        assert!(query.contains("$userId: ID!"));
    }
    for (query, variables) in queries
        .iter()
        .filter(|(query, _)| query.contains("ContributedTo"))
    {
        assert!(!variables.contains_key("userId"));
        assert!(!query.contains("history"));
    }
}

#[test]
fn calendar_is_requested_one_year_at_a_time_from_account_creation() {
    let github = RecordedGitHub::new();
    collect_account_snapshot(&github, LOGIN, date("2026-01-03")).unwrap();

    let calendar_windows: Vec<(String, String)> = github
        .requests
        .borrow()
        .iter()
        .filter_map(|variables| {
            Some((variables.get("from")?.clone(), variables.get("to")?.clone()))
        })
        .collect();
    assert_eq!(
        calendar_windows,
        [
            (
                "2024-11-20T00:00:00Z".to_owned(),
                "2025-11-19T23:59:59Z".to_owned()
            ),
            (
                "2025-11-20T00:00:00Z".to_owned(),
                "2026-01-02T23:59:59Z".to_owned()
            ),
        ]
    );
}

#[test]
fn a_calendar_github_cuts_short_is_requested_again_until_complete() {
    /// Days the cutting GitHub keeps from the end of each request, deliberately unlike
    /// GitHub's own limit and short enough that every year-long window gets cut.
    const DAYS_KEPT_PER_CUT_REQUEST: u64 = 100;
    let github_cutting_each_calendar = RecordedGitHub {
        calendar: |from, to| {
            let first_day_kept = from.max(to - Days::new(DAYS_KEPT_PER_CUT_REQUEST - 1));
            Ok(calendar_response(first_day_kept, to, |_| true))
        },
        ..RecordedGitHub::new()
    };
    let complete_snapshot =
        collect_account_snapshot(&RecordedGitHub::new(), LOGIN, date("2026-01-03")).unwrap();

    assert_eq!(
        collect_account_snapshot(&github_cutting_each_calendar, LOGIN, date("2026-01-03")),
        Ok(complete_snapshot)
    );
}

#[test]
fn the_day_in_progress_is_left_out_even_when_github_reports_it() {
    let github = RecordedGitHub {
        calendar: |from, _| Ok(calendar_response(from, date("2026-01-03"), |_| true)),
        ..RecordedGitHub::new()
    };
    let account_snapshot = collect_account_snapshot(&github, LOGIN, date("2026-01-03")).unwrap();

    let contribution_history = account_snapshot.contribution_history.unwrap();
    assert_eq!(contribution_history.observed_on, date("2026-01-02"));
    assert_eq!(
        contribution_history.days.last().unwrap().date,
        date("2026-01-02")
    );
}

#[test]
fn an_account_created_today_has_an_empty_history_through_yesterday() {
    let github = RecordedGitHub {
        account: json!({ "id": "U_synthetic", "databaseId": 4242, "createdAt": "2026-01-03T08:00:00Z" }),
        ..RecordedGitHub::new()
    };
    let account_snapshot = collect_account_snapshot(&github, LOGIN, date("2026-01-03")).unwrap();

    assert!(
        github
            .requests
            .borrow()
            .iter()
            .all(|variables| !variables.contains_key("from"))
    );
    let contribution_history = account_snapshot.contribution_history.unwrap();
    assert_eq!(contribution_history.observed_on, date("2026-01-02"));
    assert!(contribution_history.days.is_empty());
}

#[test]
fn gh_failures_pass_through_unchanged() {
    let github = RecordedGitHub {
        calendar: |_, _| {
            Err(CollectionError::GhFailed(
                "HTTP 401: Bad credentials".to_owned(),
            ))
        },
        ..RecordedGitHub::new()
    };
    assert_eq!(
        collect_account_snapshot(&github, LOGIN, date("2026-01-03")),
        Err(CollectionError::GhFailed(
            "HTTP 401: Bad credentials".to_owned()
        ))
    );
}

#[test]
fn unknown_accounts_fail_instead_of_producing_an_empty_snapshot() {
    let github = RecordedGitHub {
        account: Value::Null,
        ..RecordedGitHub::new()
    };
    assert_eq!(
        collect_account_snapshot(&github, LOGIN, date("2026-01-03")),
        Err(CollectionError::AccountNotFound(LOGIN.to_owned()))
    );
}

#[test]
fn missing_repository_fields_fail_instead_of_guessing() {
    let mut github = RecordedGitHub::new();
    github.owned_pages[1]["nodes"][0]
        .as_object_mut()
        .unwrap()
        .remove("isFork");
    match collect_account_snapshot(&github, LOGIN, date("2026-01-03")) {
        Err(CollectionError::UnexpectedResponse(detail)) => assert!(detail.contains("isFork")),
        other => panic!("expected an unexpected-response error, got {other:?}"),
    }
}

#[test]
fn two_colors_for_one_language_fail() {
    let mut github = RecordedGitHub::new();
    github.owned_pages[1]["nodes"][0]["primaryLanguage"]["color"] = json!("#000000");
    assert_eq!(
        collect_account_snapshot(&github, LOGIN, date("2026-01-03")),
        Err(CollectionError::ConflictingLanguageColor("Rust".to_owned()))
    );
}

#[test]
fn calendar_gaps_fail_instead_of_shortening_the_history() {
    let github = RecordedGitHub {
        calendar: |from, to| Ok(calendar_response(from, to, |day| day != date("2025-06-01"))),
        ..RecordedGitHub::new()
    };
    assert_eq!(
        collect_account_snapshot(&github, LOGIN, date("2026-01-03")),
        Err(CollectionError::InvalidHistory(
            ContributionHistoryError::MissingDate(date("2025-06-01"))
        ))
    );
}

#[test]
fn a_calendar_starting_after_account_creation_fails() {
    let github = RecordedGitHub {
        calendar: |from, to| Ok(calendar_response(from, to, |day| day >= date("2024-12-01"))),
        ..RecordedGitHub::new()
    };
    assert_eq!(
        collect_account_snapshot(&github, LOGIN, date("2026-01-03")),
        Err(CollectionError::IncompleteCalendar {
            created_on: date("2024-11-20"),
            first_reported_on: Some(date("2024-12-01")),
        })
    );
}

#[test]
fn command_line_usage_errors_exit_two_and_help_exits_zero() {
    for invalid_arguments in [
        &[][..],
        &["collect"],
        &["collect", "--output", "x"],
        &["render"],
    ] {
        let command_output = run_git_koi(invalid_arguments, None);
        assert_eq!(command_output.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&command_output.stderr).starts_with("Usage:"));
    }
    let help_output = run_git_koi(&["--help"], None);
    assert_eq!(help_output.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&help_output.stdout).contains("GH_TOKEN"));
}

#[test]
fn collection_failure_exits_one_and_keeps_the_previous_snapshot() {
    let scratch_directory =
        std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("collect-failure");
    let empty_path_directory = scratch_directory.join("empty-path");
    fs::create_dir_all(&empty_path_directory).unwrap();
    let output_path = scratch_directory.join("snapshot.json");
    fs::write(&output_path, "previous snapshot\n").unwrap();

    let command_output = run_git_koi(
        &[
            "collect",
            "octocat",
            "--output",
            output_path.to_str().unwrap(),
        ],
        Some(empty_path_directory.to_str().unwrap()),
    );

    assert_eq!(command_output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&command_output.stderr).contains("gh is not installed"));
    assert!(command_output.stdout.is_empty());
    assert_eq!(
        fs::read_to_string(&output_path).unwrap(),
        "previous snapshot\n"
    );
    assert!(!output_path.with_extension("tmp").exists());
}

/// Answers queries the way GitHub does for a small synthetic account and records
/// the variables of every request.
struct RecordedGitHub {
    account: Value,
    owned_pages: Vec<Value>,
    contributed_pages: Vec<Value>,
    calendar: fn(NaiveDate, NaiveDate) -> Result<Value, CollectionError>,
    requests: RefCell<Vec<BTreeMap<String, String>>>,
    queries: RefCell<Vec<(String, BTreeMap<String, String>)>>,
}

impl RecordedGitHub {
    fn new() -> Self {
        Self {
            account: json!({ "id": "U_synthetic", "databaseId": 4242, "createdAt": "2024-11-20T10:15:00Z" }),
            owned_pages: vec![
                repository_page(
                    vec![
                        repository_node(1, false, Some("Rust")),
                        repository_node(2, false, Some("Python")),
                    ],
                    Some("page-1"),
                ),
                repository_page(vec![repository_node(3, true, Some("Rust"))], None),
            ],
            contributed_pages: vec![repository_page(vec![repository_node(4, false, None)], None)],
            calendar: |from, to| Ok(calendar_response(from, to, |_| true)),
            requests: RefCell::new(Vec::new()),
            queries: RefCell::new(Vec::new()),
        }
    }
}

impl GraphqlClient for RecordedGitHub {
    fn run_query(&self, query: &str, variables: &[(&str, &str)]) -> Result<Value, CollectionError> {
        let variables: BTreeMap<String, String> = variables
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        self.requests.borrow_mut().push(variables.clone());
        self.queries
            .borrow_mut()
            .push((query.to_owned(), variables.clone()));
        if query.contains("contributionsCollection") {
            let window_date = |name: &str| date(&variables[name][..10]);
            return (self.calendar)(window_date("from"), window_date("to"));
        }
        let (connection_name, pages) = if query.contains("repositoriesContributedTo") {
            ("repositoriesContributedTo", &self.contributed_pages)
        } else if query.contains("repositories(") {
            ("repositories", &self.owned_pages)
        } else {
            return Ok(json!({ "user": self.account }));
        };
        let page_index = variables.get("cursor").map_or(0, |cursor| {
            cursor.trim_start_matches("page-").parse::<usize>().unwrap()
        });
        Ok(json!({ "user": { connection_name: pages[page_index] } }))
    }
}

fn repository_node(repository_id: u64, is_fork: bool, language: Option<&str>) -> Value {
    let language_color = |name: &str| match name {
        "Rust" => "#dea584",
        "Python" => "#3572A5",
        _ => panic!("no recorded color for {name}"),
    };
    json!({
        "databaseId": repository_id,
        "visibility": "PUBLIC",
        "isFork": is_fork,
        "primaryLanguage": language.map(|name| json!({ "name": name, "color": language_color(name) })),
        "defaultBranchRef": { "target": { "history": { "totalCount": 1 } } },
    })
}

/// An owned repository node whose default branch holds `commits` of the owner's commits.
fn owned_node(repository_id: u64, commits: u64) -> Value {
    let mut node = repository_node(repository_id, false, None);
    node["defaultBranchRef"]["target"]["history"]["totalCount"] = json!(commits);
    node
}

fn repository_page(nodes: Vec<Value>, next_cursor: Option<&str>) -> Value {
    json!({
        "nodes": nodes,
        "pageInfo": { "hasNextPage": next_cursor.is_some(), "endCursor": next_cursor },
    })
}

/// A calendar the way GitHub returns it: whole weeks starting on the Sunday on or before
/// `from`, so the first week can include days before the requested window.
fn calendar_response(from: NaiveDate, to: NaiveDate, keep_day: fn(NaiveDate) -> bool) -> Value {
    let first_sunday = from - Days::new(u64::from(from.weekday().days_since(Weekday::Sun)));
    let calendar_days: Vec<NaiveDate> = first_sunday
        .iter_days()
        .take_while(|day| *day <= to)
        .collect();
    let weeks: Vec<Value> = calendar_days
        .chunks(7)
        .map(|week_days| {
            let contribution_days: Vec<Value> = week_days
                .iter()
                .filter(|day| keep_day(**day))
                .map(|day| json!({ "date": day.to_string(), "contributionCount": synthetic_count(*day) }))
                .collect();
            json!({ "contributionDays": contribution_days })
        })
        .collect();
    json!({ "user": { "contributionsCollection": { "contributionCalendar": { "weeks": weeks } } } })
}

fn synthetic_count(day: NaiveDate) -> u32 {
    day.ordinal() % 4
}

fn date(text: &str) -> NaiveDate {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
}

fn run_git_koi(arguments: &[&str], path_override: Option<&str>) -> std::process::Output {
    let mut git_koi = Command::new(env!("CARGO_BIN_EXE_git-koi"));
    git_koi.args(arguments);
    if let Some(path_override) = path_override {
        git_koi.env("PATH", path_override);
    }
    git_koi.output().unwrap()
}
