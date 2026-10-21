# Using Repo Terrarium

Repo Terrarium is a pixel-art terrarium that lives along one edge of a monitor and grows from a
git repository on your computer. This guide covers installing it, connecting a repository, and
reading what it shows.

## Installing

**On a Mac** with Apple silicon and macOS 13 or later, open the DMG and drag Repo Terrarium onto
the Applications folder beside it.

The app isn't notarized by Apple yet, so the first time you open it, macOS says it can't check
it and won't open it. Open System Settings, go to Privacy & Security, scroll down to where it
names Repo Terrarium and click Open Anyway, then confirm. From then on it opens normally. It
doesn't appear in the Dock.

**On Windows** 10 or 11 (64-bit), run the installer. It installs for you alone, with no pages to
click through, and opens the app. It isn't signed yet either, so SmartScreen may stop it with
"Windows protected your PC": click More info, then Run anyway.

## Connecting a repository

The first time it runs, Repo Terrarium asks for a repository. Choose a folder that holds a git
repository and click Connect. Leave "Open at login" ticked if you want the strip back each time
you log in. The window then lists what it found: coverage, test and dead-code reports, and CI
workflows. A large history takes a moment to read, and the strip's label shows how far it got.

If the repository is in Documents, Desktop or Downloads, or on another drive, macOS first asks
whether Repo Terrarium may read that folder.

Repo Terrarium only reads your repository and never changes anything in it, and nothing leaves
your computer unless you connect GitHub Actions for CI weather.

It shows one repository at a time. To switch, disconnect it in Settings, under Repository, and
connect another.

## The strip

The strip runs along the bottom of a monitor, or the top if you choose. On a Mac it sits beneath
your windows, like the desktop; Control-Option-T lifts it above them for ten seconds. On Windows
it reserves its edge of the screen, as the taskbar does, and steps aside for full-screen apps.

Right-click it for Settings, Time-lapse, Pause and Quit. Pause holds everything still while the
terrarium keeps its own time; Resume picks up where things are now. Opening the app again while
it runs shows its settings.

## Reading the terrarium

The repository sets the terrarium's climate:

- **Light** is test coverage, from a coverage report.
- **Soil** is code health: dead code, complexity and TODO density.
- **Rain** is how often you commit, against the repository's own usual pace. **Wind** is how
  much each day's changes touch.
- **Weather** is your tests or CI: clear when they pass, a storm when they fail, and haze when
  nothing says.

Light and soil follow the default branch. Reports written on another branch are kept and shown
in the soil panel, but don't change the plants. Rain counts commits on every local branch.

The plants follow the weaker of light and soil. Low light makes them pale and leggy; poor soil
makes them wilt, with weeds, litter and fungus at their feet. A reading that is missing counts
as middling, so a repository without reports looks neither good nor bad.

Over time the terrarium settles into one of six states:

- **Thriving:** healthy and active. Lush, and the residents are busy.
- **Mature:** healthy but quiet. Large, settled plants and slow motion.
- **Struggling:** unhealthy. Pale or wilting plants and weeds.
- **Dormant:** no commits for 30 days, while healthy. Plants still and grey-green, residents
  asleep and condensation on the glass, but nothing rots.
- **Dead:** no commits for 30 days, while unhealthy. Bleached plants, litter and fungus, and
  after a year, dust on the glass.
- **Recovering:** health climbing back after a bad stretch. Fresh growth among the litter.

Things that happen in the repository play out once, one at a time:

- Tests or CI failing bring a storm in from one side; passing again clears it.
- Three days of failing bring lightning, which scorches a plant.
- A version tag, like `v1.2`, makes the plants flower for a day. A new major version also carves
  a mark into a standing stone.
- The first commit after two quiet weeks brings rain.
- A force push to the default branch cracks the ground.
- Deleting a whole directory of 500 lines or more opens a sinkhole and leaves a fossil.
- A day of far more change than usual blows a windstorm through.
- Coverage going up or down brightens or dims the light for a week.
- Clearing out a fifth or more of the dead code burns off the weeds.

A standing stone rises for each year of the repository's age, up to three, and weathers as it
gets older. Hover a stone for its year, or a fossil for the directory it came from.

Day and night follow your clock: day from 7:00, night from 19:00, with 45 minutes of dusk or
dawn between. At night some plants and fungi glow, and fireflies, if yours has them, light up.
After 30 minutes with no input anywhere on the computer, the strip dims and slows down.

The label in the corner shows, in turn, the repository's name, the branch and its weather, and
a note when there is one:

- `history 40%` while the history is read;
- `settling 9 days` for a repository younger than two weeks;
- `dormant since jun`;
- `repo not found` when the folder has moved: relink it in Settings, under Repository;
- `halted` if something went wrong inside: the strip stops changing until you restart it.

## The soil panel

Hover the soil along the bottom of the strip and a panel rises with the climate: light, soil,
rain, wind, weather, terrain (lines of code) and age, each with its value and which way it went
over the past week. A small icon marks a reading that is stale (the default branch has moved on
since the report was written) or unknown.

Click to pin the panel, and click a row with a pointer to see what's behind it: soil opens dead
code, complexity and TODO density, and each of those opens the files that weigh most. Click the
title to go back, or anywhere off the panel to close it.

## More signals

The git history works on its own. Everything else comes from files your tools already write.
Repo Terrarium looks in the usual places and reads the newest file it finds.

- **Coverage:** lcov (`coverage/lcov.info`, `lcov.info`, `coverage.lcov`) or Cobertura XML
  (`coverage/cobertura-coverage.xml`, `coverage.xml`, `cobertura.xml`).
- **Tests:** JUnit XML (`junit.xml`, `test-results.xml`, `target/nextest/*/junit.xml` and a few
  more). Or run your tests through Repo Terrarium, which records whether they passed:

  ```
  repo-terrarium record-tests -- npm test
  ```

  On a Mac the command is inside the app, at
  `/Applications/Repo Terrarium.app/Contents/MacOS/repo-terrarium`; on Windows it is
  `%LOCALAPPDATA%\Programs\repo-terrarium\repo-terrarium.exe`. Nothing is written into the
  repository.
- **Dead code:** a Knip report in `knip-report.json`
  (`npx knip --reporter json --no-progress > knip-report.json`). Or tick "Run Knip from this
  repository" in Settings, under Adapters, and the repository's own Knip runs after each new
  commit on the default branch. Knip may write its cache into `node_modules`.
- **Complexity and TODO density** are measured from the TypeScript, JavaScript, Python, Rust and
  Go on the default branch, with nothing to set up.
- **CI weather** from GitHub Actions: in Settings, under Adapters, save a fine-grained token with
  read access to Actions. It's kept in the keychain (on Windows, the Credential Manager), and
  only api.github.com is asked, once a minute.

Settings, under Adapters, lists what was found and how to connect what wasn't, and takes a
report path if yours lives somewhere else. `repo-terrarium discover <path>` prints the same list
in a terminal.

## Time-lapse, export and import

Right-click and choose Time-lapse, or press Control-Option-H (Ctrl+Alt+H on Windows), to watch
the repository's whole history in 30 seconds. Pick a range, day or night and a scale, then save
it as a GIF, as a PNG postcard of the day shown, or as an MP4 if ffmpeg is installed.

The git history replays on any computer, but light, soil and weather are recorded only on the
one that saw them. To take them to another computer, export the observations in Settings, under
Repository, and import them there into the same repository. Export climate saves each day's
readings and states as JSON.

## Settings

- **Repository:** where it is (Locate and Relink if you moved it); New terrarium, which lays the
  same history out afresh so plants, residents and stones take new places; Disconnect; export
  and import.
- **Strip:** the display, the edge, and the size: the largest that fits, or one step smaller.
- **Adapters:** the reports, Knip and GitHub Actions, as above.
- **Hotkeys:** the keys for lifting the strip and for the time-lapse.
- **Time:** when day and night begin, and how long until the strip dims.
- **General:** open at login, and a crash log you can turn on.

## Privacy

Repo Terrarium reads your repository and never writes to it. It runs nothing from it except Knip,
and only when you ask. It has no accounts, no telemetry and no update checks. The only network
use is the GitHub Actions poll, once you save a token.

Everything it keeps is in one folder: `~/Library/Application Support/io.github.faircat-gd.repo-terrarium`
on a Mac, `%LOCALAPPDATA%\io.github.faircat-gd.repo-terrarium` on Windows. The crash log, if you
turn it on, is written there and sent nowhere.

## Updating and uninstalling

To update, install the new version over the old one. Because the Mac app isn't signed with an
Apple Developer ID yet, macOS may ask again after an update for folder access, the keychain and
opening at login.

To uninstall on a Mac, quit from the strip's menu and move Repo Terrarium from Applications to
the Trash. On Windows, uninstall it from Settings, under Apps; that quits the strip and gives its
edge of the screen back. Either way the data folder stays, in case you come back, and so does a
saved GitHub token: remove the token in Settings, under Adapters, and delete the folder to leave
nothing behind.
