# Pond: lilypads and a wide swim

Status: approved design, awaiting spec review. Target: git-koi 2.0.0, released
from the `Mehulsri07/git-koi` fork.

## Goal

An opt-in pond banner for a profile README. The koi swims the full width of the
README column, under lilypads. Each lilypad stands for one of the account's own
public repositories, and it grows with the owner's commits to that repository.
Without the opt-in, every output stays byte-identical to 1.0.2.

Success means:

- `git-koi render --pond` and the Action input `pond: true` produce `koi.gif` at
  832×384, which shows pixel-exact at `width="832"`.
- The GIF for a maximum-size koi with 12 pads stays under 1 MB.
- The same snapshot always produces the same bytes.

## Decisions

| Topic            | Decision                                                                                                                                                                                                                                         |
| ---------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| What a pad is    | One public, non-fork repository the account **owns**                                                                                                                                                                                             |
| Pad cap          | 12. Past 12, keep the repositories with the most commits; ties go to the lower repository ID                                                                                                                                                     |
| Pad size         | The owner's commits on the default branch: ≤10 → size 1, 11–20 → 2, 21–40 → 3, 41–80 → 4, 81+ → 5                                                                                                                                                |
| Pad diameters    | Sizes 1–5 = 16, 24, 32, 40, 48 native pixels; no 64                                                                                                                                                                                              |
| Flowers          | Rank languages by how many pads they have (ties by name). The biggest pad of language #1 blooms pink, #2 white, #3 purple. Each language ranked #4 or lower gets one bud on its biggest pad. Pads with no language and all other pads stay plain |
| Drawing          | Procedural, like the koi: notched disc, shading lit from the top left, veins, droplets on sizes 4–5, one still ripple arc per pad                                                                                                                |
| Palette          | Only the colors listed under Palette                                                                                                                                                                                                             |
| Layering         | Pads draw above the koi; the koi swims under them                                                                                                                                                                                                |
| Placement        | Seeded from the account ID. Pads never overlap each other and keep a 2-pixel gap. The layout changes only when the set of pads or a pad's size changes                                                                                           |
| Canvas           | 416×192 native, exported at 2× = 832×384                                                                                                                                                                                                         |
| Swim             | A racetrack course, about 180 frames per loop                                                                                                                                                                                                    |
| Rest and feeding | Hover course, centered half under the biggest pad                                                                                                                                                                                                |
| Encoding         | One base frame with the pads, then koi frames that each restore to the previous frame                                                                                                                                                            |
| Opt-in           | `--pond` flag and `pond` Action input, off by default                                                                                                                                                                                            |
| Versioning       | 2.0.0: the snapshot schema changes, so 1.x snapshots must be re-collected                                                                                                                                                                        |

## Palette

Hard-edged pixels only: the encoder rejects partial transparency.

- Pad base: `#3a5f4b` `#5a8a4a` `#6f9a45` `#a8c24a`
- Pad shade: `#24483c` `#3f6a46` `#56853f`
- Pad highlight / rim: `#f2c94c` `#f5d565` `#f8e7a0`
- Lotus pink: `#e86f9a` `#f08fb0` `#f7b8cc` `#fbeee0`
- Lotus white: `#a993c6` `#c4b2d9` `#e3d9ec` `#fbeee0`
- Lotus purple: `#7a4fa3` `#a275c9` `#c9a8e3` `#f1e6f2`
- Pollen: `#f0a032` `#f5b93c` `#f8de6a`
- Ripple and droplet: `#6fb3b8` `#a8e3e6`. The two dark water teals from the
  reference are dropped because they vanish on GitHub's dark background.

The values are read off the reference sheet and get tuned once against real
renders. The budget is about 33 pond colors plus the koi's maximum of 22 today,
well under 255.

## Data: snapshot v2

`collect` asks GitHub for more data on the owned repositories connection:

```graphql
nodes {
  databaseId visibility isFork primaryLanguage { name color }
  defaultBranchRef { target { ... on Commit {
    history(author: { id: $userId }) { totalCount } } } }
}
```

`$userId` is the user's node `id`, which the account query now also returns. A
repository with no default branch has 0 commits.

Snapshot version 4 adds one field to `AccountSnapshot`:
`owned_repositories: Option<Vec<OwnedRepository>>`, where
`OwnedRepository { repository_id: u64, owner_commit_count: u64 }` lists every
public, non-fork repository the account owns. `RepositoryObservation` is
unchanged. Versions 1 to 3 still load, with `owned_repositories: None`, and
still render the normal koi. `--pond` needs version 4 and fails with "the pond
needs a snapshot collected by git-koi 2.0; run git-koi collect again". Adding a
public field to `AccountSnapshot` is the breaking change behind 2.0. Language
shares are unchanged.

## Modules

**`history`**: the new fields, their (de)serialization, and the error message
for 1.x snapshots.

**`pond`** (new, pure):

- `PondLayout { pads: Vec<Pad> }`
- `Pad { center: [f64; 2], size: PadSize, bloom: Option<Bloom>, notch_radians: f64 }`
- `PadSize` runs from 1 to 5. `Bloom` is `Pink`, `White`, `Purple` or `Bud`.
- `layout_pond(snapshot, identity_seed) -> PondLayout`. It selects the pads,
  sizes them, assigns flowers, and places them.

Placement:

- Pads are placed from largest to smallest. Each pad tries seeded candidate
  points spread across the width, with a bias toward small pads near large ones.
- A candidate is rejected when the pad would overlap another pad or cross the
  canvas edge.
- When no candidate fits after a fixed number of tries, the pad drops one size.
  A size-1 pad that still does not fit is left out, and a test covers that path.
- The seed for each pad is the account ID together with the repository ID, so
  adding a repository does not move the others.

**`motion`**: `Course::Track { progress }` is a racetrack with two straight runs
and semicircle ends.

- The end radius is the lap radius rule from `calculate_lap_radius`, adapted to
  the pond height.
- Speed follows the existing stroke-and-glide drive, so heading and body bend
  come from the track's tangent and curvature.
- Rest, deep rest and surface feeding keep their hover course, centered under
  the biggest pad.
- `Course` gains a variant, which breaks exhaustive matches; that is acceptable
  in 2.0.

**`render`**:

- `KoiFrame` carries its own width and height, with public `width()` and
  `height()` methods. The square 256 frame stays the default.
- `render_pond_frame(development, body, behavior, &PondLayout, frame_index)`
  draws the koi, applies its existing depth tint, and then draws the pads over
  it.
- `render_pond_base(&PondLayout)` draws the pads alone.

**`output`**:

- The encoders read the frame size instead of the 256 constants.
- `encode_pond_gif(base, koi_frames, pads)`:
  - Frame 0 is the base: the box around the pads, disposal `None`, a 2/100 s
    delay.
  - Every koi frame stores only the box around the koi, including the pad pixels
    that cover it, with transparent pixels elsewhere. Its disposal is
    `RestoreToPrevious`.
  - Known cost: the koi is absent for that one 20 ms base frame in each loop.
    Delays of 1/100 s or less are clamped to 100 ms by browsers, so 2/100 s is
    the floor.
- Without `--pond`, output is unchanged.

## Command line and Action

- `git-koi render SNAPSHOT --output-dir DIR [--body BODY] [--pond]`
- With `--pond`: `koi.gif` is 832×384, `koi.png` is the base frame plus koi
  frame 0, and `preview.html` shows the banner.
- Action input `pond`, default `false`.
- `examples/profile-workflow.yml` documents `pond: true` with `width="832"`.

## Release from the fork

- `Cargo.toml` `repository` → `https://github.com/Mehulsri07/git-koi`.
  `action.yml` downloads releases from that repository.
- In `release.yml`, the crates.io publish steps are skipped. The crate name
  belongs to upstream. Releases are GitHub releases only.
- The version goes to 2.0.0, with CHANGELOG entries for the pond, the snapshot
  v2 break, and the fork origin.
- Mehulsri07's profile workflow moves to `Mehulsri07/git-koi@v2` with
  `pond: true`. The README embed changes to `width="832"`.

## Testing

All tests sit at public boundaries, as the existing suite does.

- Pad size boundaries at 10/11, 20/21, 40/41 and 80/81 commits.
- The cap: 13 owned repositories give 12 pads, and the one with the fewest
  commits is dropped. Forks, private repositories and contributed repositories
  never get a pad.
- Flowers: the pink, white and purple assignment by language rank, buds for
  language #4 and lower, and plain pads with no language.
- Determinism: the same snapshot gives an identical layout and identical GIF
  bytes. Adding a repository leaves the other pads' centers in place.
- Geometry: no pad overlaps another or leaves the canvas, at 12 size-5 pads in
  the worst case.
- Encoding: every frame stays at or under 255 colors with no partial alpha. The
  maximum koi with 12 size-5 pads stays under the 1 MB budget.
- Rest and feeding frames stay inside the canvas.
- Regression: without `--pond`, 1.0.2 fixtures render byte-identical outputs.
- Snapshots v1 to v3 still load and render the normal koi; `--pond` on one fails
  with the re-collect message.
- The review library (`examples/generate-library.rs`) gains pond sheets on the
  light and dark backgrounds.

## Out of scope

- Animated ripples and pads that drift.
- Bloom or bud showing recent activity, which would need `pushedAt`.
- Publishing to crates.io from the fork.
- Sending this upstream.
