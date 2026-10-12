# terrarium-engine

The headless live pipeline, run on a worker thread.

## Owns

- **Connect** (`live`): `--repo <path>` creates the terrarium or relinks the one with the
  repository's seed, derived as `record-tests` derives it; a plain run takes the one
  `repo.active` names, with its recorded default branch, unless it was disconnected.
- **The lifecycle** (`lifecycle`): relinking a terrarium to a repository that moved (only the
  same repository), re-seeding (a new install salt, the cache dropped, the observations kept),
  and disconnecting, with or without deleting the observations. The app runs each with the
  engine stopped.
- **Refresh** (`live`): git, then the reports, then replay. A report is read only once it has
  been still for 500 ms, and recorded only when its content changed since its format's last
  observation. A default branch moved without fast-forwarding is recorded as a `ref_rewrite`, with what git
  read of the commits it took off the chain; a move made while the app was closed is read back
  from the object database at the next start.
  Replay resumes the cached state before the open hour when truth accepts it, and replays in
  full otherwise; the cache is written about once an hour.
- **The worker** (`worker`): the thread, its channel and its sink. A panic in a refresh is
  caught: the scene freezes and `Halted` is sent. Shutting down stops a history read in
  progress; a join waits out a replay too, before a lifecycle change.
- **What the label notes once** (`live`): a simulation-version change and a recorded history
  rewrite, each held until a snapshot carries it.
- **When to refresh** (`schedule`): 500 ms after the watcher's last event, a follow-up for a
  report still being written, the wall clock's hour changing (the hourly tick, and any wake or
  clock jump), a wake from the platform, and a retry once a minute while the repository is
  missing.
- **What to watch** (`watch`): the git directories, the refs and reflogs, the directories
  reports are written into, and the recorded runs' directory; registered in one batch.
- **The clock** (`clock`): the wall clock, behind a trait so tests set it.
- The headless replay (`headless`): read a repository, import an observations export into a
  store in a directory the caller owns, and dump truth's replay, or replay it a day at a time
  into each day's scene. `repo-terrarium replay --dump`, `cargo xtask golden` and `cargo xtask
  arc` run through it.
- Which event a scene plays (`showing`): the best not yet shown, held through the next hour
  unless outranked; the live view, the day-by-day replay and the debug window's simulation
  all choose it there. What the live scenes had the chance to play is kept across restarts
  (`shown`), so a restart within an event's hour does not play it again.
- Observations (`observe`): a report an adapter read, HEAD and a time become the store's
  record; a Knip report becomes dead code weighed against the exports the source metrics
  counted in HEAD's commit; a measurement of the source becomes complexity and TODO density;
  and a ref rewrite becomes its record. Only a known reading is recorded.
- **The source** (`live`): the committed source at the default tip is measured once per tip,
  while HEAD is at it, parsing only the files not met before in the session. A session's first
  refresh leaves it to an immediate follow-up, so the first snapshot never waits on a parse.
  One measurement serves the tip's metrics and a Knip report's exports in the same refresh.
- **Observations of one second:** replay orders the observations of one second by their merge
  key, not by when they were recorded, and truth keeps the last reading of each kind and follows
  the newest observation's HEAD. So an observation recorded in the same second as one at
  another head is dated a second later, and the newer is the one held.
- **Jobs** (`ci`, `knip`): the GitHub Actions poll and the opt-in Knip run each run on a thread
  of their own and come back to the worker, which owns them; a refresh records what they
  brought. The poll reads the token from the keychain once, asks HEAD's branch then the default
  one every minute (longer while GitHub cannot be reached or limits the rate, never while it
  refuses the token), and records a branch's runs when they change, or when HEAD's branch is not
  the one truth last heard of. Runs that can no longer be read are recorded as unknown, so the
  local results take over. Knip runs once per default tip while HEAD is there, only on the
  opt-in given for this repository; its report is weighed as a Knip report's, and only while
  HEAD is still at the tip it ran at.

## Why it is a crate of its own

Without UI types, the pipeline can be driven from tests and from `cargo xtask`, which is how the
golden replay and the integration tests run. It never resolves the data directory itself: its
caller passes one, so tests use their own.

## Rules

- It depends on `terrarium-truth`, `terrarium-adapters` and `terrarium-store`, and on no other
  workspace crate. It has no UI types.
- Only the refresh path calls adapters. Replay never runs a tool (I-4).
- Nothing here writes into the repository. Nothing runs from it but Knip, on the opt-in given
  for that repository, whose own cache writes are the one exemption from N-10 (I-5).
- No connection is made without a `CiConfig`, a token and a GitHub repository (N-09).

## Requirements

N-04, N-06, N-09 and N-10; invariants I-4, I-5, I-6 and I-16.
