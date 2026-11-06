use git_koi::history::{
    AccountSnapshot, ConflictingRepository, OwnedRepository, RepositoryObservation,
    RepositoryVisibility,
};
use git_koi::pond::{
    Bloom, MAX_PADS, POND_HEIGHT_PIXELS, POND_WIDTH_PIXELS, PadSize, PondError, layout_pond,
};

fn observation(
    repository_id: u64,
    visibility: RepositoryVisibility,
    is_fork: bool,
    language: Option<&str>,
) -> RepositoryObservation {
    RepositoryObservation {
        repository_id,
        visibility,
        is_fork,
        primary_language: language.map(str::to_owned),
    }
}

fn public(repository_id: u64, language: Option<&str>) -> RepositoryObservation {
    observation(repository_id, RepositoryVisibility::Public, false, language)
}

fn owned(repository_id: u64, owner_commit_count: u64) -> OwnedRepository {
    OwnedRepository {
        repository_id,
        owner_commit_count,
    }
}

fn snapshot(
    account_id: u64,
    repositories: Vec<RepositoryObservation>,
    owned_repositories: Option<Vec<OwnedRepository>>,
) -> AccountSnapshot {
    AccountSnapshot {
        account_id,
        repositories,
        contribution_history: None,
        language_colors: None,
        owned_repositories,
    }
}

fn ids(layout: &git_koi::pond::PondLayout) -> Vec<u64> {
    let mut ids: Vec<u64> = layout.pads.iter().map(|pad| pad.repository_id).collect();
    ids.sort();
    ids
}

#[test]
fn pad_sizes_step_at_the_commit_boundaries() {
    let expectations = [
        (0, 1, 16),
        (10, 1, 16),
        (11, 2, 24),
        (20, 2, 24),
        (21, 3, 32),
        (40, 3, 32),
        (41, 4, 40),
        (80, 4, 40),
        (81, 5, 48),
        (10_000, 5, 48),
    ];
    for (commits, level, diameter) in expectations {
        let size = PadSize::from_commits(commits);
        assert_eq!(size.level(), level, "commits {commits}");
        assert_eq!(size.diameter_pixels(), diameter, "commits {commits}");
    }
    assert_eq!(
        PadSize::from_commits(11).smaller(),
        Some(PadSize::from_commits(0))
    );
    assert_eq!(PadSize::from_commits(0).smaller(), None);
}

#[test]
fn only_owned_public_non_fork_repositories_get_pads() {
    let account_snapshot = snapshot(
        1,
        vec![
            public(1, Some("Rust")),
            observation(2, RepositoryVisibility::Public, true, Some("Rust")),
            observation(3, RepositoryVisibility::Private, false, Some("Rust")),
            public(4, Some("Rust")),
        ],
        Some(vec![owned(1, 5), owned(2, 5), owned(3, 5)]),
    );
    assert_eq!(ids(&layout_pond(&account_snapshot).unwrap()), vec![1]);
}

#[test]
fn thirteen_repositories_keep_the_twelve_with_most_commits() {
    let repositories: Vec<_> = (1..=13).map(|id| public(id, None)).collect();
    let mut owned_repositories: Vec<_> = (1..=13).map(|id| owned(id, 100 + id)).collect();
    // Repository 5 has the fewest commits.
    owned_repositories[4].owner_commit_count = 1;
    let account_snapshot = snapshot(9, repositories.clone(), Some(owned_repositories));
    let layout = layout_pond(&account_snapshot).unwrap();
    assert!(layout.pads.len() <= MAX_PADS);
    assert!(!ids(&layout).contains(&5));

    // On a tie the lower ID is kept.
    let tied: Vec<_> = (1..=13).map(|id| owned(id, 50)).collect();
    let account_snapshot = snapshot(9, repositories, Some(tied));
    let layout = layout_pond(&account_snapshot).unwrap();
    assert!(!ids(&layout).contains(&13));
    assert!(ids(&layout).contains(&12));
}

#[test]
fn flowers_follow_language_rank() {
    // Rust 4 pads, Go 2, Python 2, Kotlin 1. Ties rank by language name.
    let languages = [
        (1, "Rust", 90),
        (2, "Rust", 50),
        (3, "Rust", 30),
        (4, "Go", 20),
        (5, "Go", 60),
        (6, "Python", 15),
        (7, "Python", 14),
        (8, "Kotlin", 70),
        (9, "Rust", 5),
    ];
    let repositories: Vec<_> = languages
        .iter()
        .map(|(id, language, _)| public(*id, Some(language)))
        .collect();
    let owned_repositories: Vec<_> = languages
        .iter()
        .map(|(id, _, commits)| owned(*id, *commits))
        .collect();
    let account_snapshot = snapshot(3, repositories, Some(owned_repositories));
    let layout = layout_pond(&account_snapshot).unwrap();
    let bloom_of = |id: u64| {
        layout
            .pads
            .iter()
            .find(|pad| pad.repository_id == id)
            .map(|pad| pad.bloom)
    };
    assert_eq!(bloom_of(1), Some(Some(Bloom::Pink)));
    assert_eq!(bloom_of(5), Some(Some(Bloom::White)));
    assert_eq!(bloom_of(6), Some(Some(Bloom::Purple)));
    assert_eq!(bloom_of(8), Some(Some(Bloom::Bud)));
    for id in [2, 3, 4, 7, 9] {
        assert_eq!(bloom_of(id), Some(None), "repository {id}");
    }
}

#[test]
fn a_repository_without_a_language_gets_a_plain_pad() {
    let account_snapshot = snapshot(
        1,
        vec![public(1, None), public(2, Some("Rust"))],
        Some(vec![owned(1, 500), owned(2, 1)]),
    );
    let layout = layout_pond(&account_snapshot).unwrap();
    assert_eq!(layout.pads.len(), 2);
    for pad in &layout.pads {
        match pad.repository_id {
            1 => assert_eq!(pad.bloom, None),
            _ => assert_eq!(pad.bloom, Some(Bloom::Pink)),
        }
    }
}

#[test]
fn no_owned_repositories_gives_an_empty_pond() {
    let account_snapshot = snapshot(1, vec![public(1, Some("Rust"))], Some(vec![]));
    assert!(layout_pond(&account_snapshot).unwrap().pads.is_empty());
}

#[test]
fn a_version_3_snapshot_has_no_pond() {
    let account_snapshot = snapshot(1, vec![public(1, Some("Rust"))], None);
    let error = layout_pond(&account_snapshot).unwrap_err();
    assert_eq!(error, PondError::NoOwnedRepositories);
    assert_eq!(
        error.to_string(),
        "the pond needs a snapshot collected by git-koi 2.0; run git-koi collect again"
    );
}

#[test]
fn conflicting_observations_are_an_error() {
    let account_snapshot = snapshot(
        1,
        vec![public(1, Some("Rust")), public(1, Some("Go"))],
        Some(vec![owned(1, 5)]),
    );
    assert_eq!(
        layout_pond(&account_snapshot),
        Err(PondError::ConflictingRepository(ConflictingRepository {
            repository_id: 1
        }))
    );
}

#[test]
fn the_layout_is_the_same_every_time() {
    let repositories: Vec<_> = (1..=8).map(|id| public(id, Some("Rust"))).collect();
    let owned_repositories: Vec<_> = (1..=8).map(|id| owned(id, id * 13)).collect();
    let account_snapshot = snapshot(77, repositories, Some(owned_repositories));
    assert_eq!(
        layout_pond(&account_snapshot).unwrap(),
        layout_pond(&account_snapshot).unwrap()
    );
}

#[test]
fn adding_a_repository_keeps_the_other_pads_in_place() {
    let repositories: Vec<_> = (1..=3).map(|id| public(id, Some("Rust"))).collect();
    let owned_repositories = vec![owned(1, 100), owned(2, 30), owned(3, 15)];
    let before = snapshot(5, repositories.clone(), Some(owned_repositories.clone()));
    let mut more_repositories = repositories;
    more_repositories.push(public(4, Some("Go")));
    let mut more_owned = owned_repositories;
    more_owned.push(owned(4, 2));
    let after = snapshot(5, more_repositories, Some(more_owned));
    let before_layout = layout_pond(&before).unwrap();
    let after_layout = layout_pond(&after).unwrap();
    assert_eq!(before_layout.pads.len(), 3);
    for pad in &before_layout.pads {
        let kept = after_layout
            .pads
            .iter()
            .find(|candidate| candidate.repository_id == pad.repository_id)
            .unwrap();
        assert_eq!(kept.center, pad.center);
    }
}

#[test]
fn pads_never_overlap_or_leave_the_pond() {
    let repositories: Vec<_> = (1..=12).map(|id| public(id, Some("Rust"))).collect();
    let owned_repositories: Vec<_> = (1..=12).map(|id| owned(id, 1000)).collect();
    for account_id in 1..=50 {
        let account_snapshot = snapshot(
            account_id,
            repositories.clone(),
            Some(owned_repositories.clone()),
        );
        let layout = layout_pond(&account_snapshot).unwrap();
        for pad in &layout.pads {
            let radius = f64::from(pad.size.diameter_pixels()) / 2.0;
            assert!(pad.center[0] - radius >= 0.0);
            assert!(pad.center[0] + radius <= f64::from(POND_WIDTH_PIXELS));
            assert!(pad.center[1] - radius >= 0.0);
            assert!(pad.center[1] + radius <= f64::from(POND_HEIGHT_PIXELS));
            assert!((0.0..6.283).contains(&pad.notch_radians));
        }
        for (index, first) in layout.pads.iter().enumerate() {
            for second in &layout.pads[index + 1..] {
                let radii =
                    f64::from(first.size.diameter_pixels() + second.size.diameter_pixels()) / 2.0;
                let distance =
                    (first.center[0] - second.center[0]).hypot(first.center[1] - second.center[1]);
                assert!(distance >= radii + 2.0, "account {account_id}");
            }
        }
    }
}
