//! The source metrics' budgets: measuring a fictional codebase of more than 100,000 lines of
//! code in the six tier-one languages, committed in one repository, takes under [`COLD`] from
//! nothing; after a commit that changes one file, measuring again parses that file alone and
//! takes under [`ONE_FILE`]. Run by `cargo xtask bench`.
//!
//! The repository is generated into a temporary directory first, untimed, and measured once
//! untimed with metrics of its own, so the file system has settled the loose objects it was
//! just handed; the timed measurement starts from nothing again.

#![forbid(unsafe_code)]

use std::fmt::Write as _;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use terrarium_adapters::fixture::{Edit, FixtureRepo, Stamp};
use terrarium_adapters::git::GitRepo;
use terrarium_adapters::metrics::{Language, Measurement, Metrics};

const COLD: Duration = Duration::from_secs(1);
const ONE_FILE: Duration = Duration::from_millis(50);
const LINES: u64 = 100_000;
/// Files per language; each holds [`FUNCTIONS`] functions.
const FILES: usize = 180;
const FUNCTIONS: usize = 9;

/// One fictional function of `language`, named by file `f` and function `k`.
fn function(language: Language, f: usize, k: usize) -> String {
    match language {
        Language::TypeScript => format!(
            "// Bed {f}, row {k}.\n\
             export function water{f}_{k}(plants: number[], dry: boolean): number {{\n\
             \x20 let poured = 0;\n\
             \x20 for (const plant of plants) {{\n\
             \x20   if (plant > {k} && !dry) {{\n\
             \x20     poured += plant ?? 1;\n\
             \x20   }} else if (plant < 0) {{\n\
             \x20     poured -= 1; // TODO: drain\n\
             \x20   }}\n\
             \x20 }}\n\
             \x20 return poured;\n\
             }}\n\n"
        ),
        Language::Tsx => format!(
            "export const Sprout{f}_{k} = ({{ tall }}: {{ tall: boolean }}) => {{\n\
             \x20 const leaves = tall ? {k} : 1;\n\
             \x20 if (!leaves) {{\n\
             \x20   return null;\n\
             \x20 }}\n\
             \x20 return (\n\
             \x20   <div className={{tall ? \"tall\" : \"short\"}}>\n\
             \x20     {{leaves}}\n\
             \x20   </div>\n\
             \x20 );\n\
             }};\n\n"
        ),
        Language::JavaScript => format!(
            "export function settle{f}_{k}(pot) {{\n\
             \x20 pot.soil ??= \"loam\";\n\
             \x20 while (pot.water > {k}) {{\n\
             \x20   pot.water -= 1;\n\
             \x20 }}\n\
             \x20 switch (pot.kind) {{\n\
             \x20   case \"clay\":\n\
             \x20     return 1;\n\
             \x20   default:\n\
             \x20     return pot.water || 0;\n\
             \x20 }}\n\
             }}\n\n"
        ),
        Language::Python => format!(
            "# FIXME: bed {f}\n\
             def feed_{f}_{k}(fish, hungry):\n\
             \x20   if hungry and fish:\n\
             \x20       return [x for x in fish if x > {k}]\n\
             \x20   elif not fish:\n\
             \x20       return []\n\
             \x20   total = 0\n\
             \x20   while hungry:\n\
             \x20       total += 1\n\
             \x20       hungry = total < {k}\n\
             \x20   return total\n\n\n"
        ),
        Language::Rust => format!(
            "/// Bed {f}, row {k}.\n\
             pub fn sprout_{f}_{k}(seeds: &[u32], warm: bool) -> u32 {{\n\
             \x20   let mut grown = 0;\n\
             \x20   for seed in seeds {{\n\
             \x20       if warm || *seed > {k} {{\n\
             \x20           grown += 1;\n\
             \x20       }} else {{\n\
             \x20           grown += match seed {{ 0 => 0, 1 => 1, _ => 2 }};\n\
             \x20       }}\n\
             \x20   }}\n\
             \x20   grown\n\
             }}\n\n"
        ),
        Language::Go => format!(
            "// Water{f}x{k} waters one bed.\n\
             func Water{f}x{k}(beds []int, rain bool) int {{\n\
             \ttotal := 0\n\
             \tfor _, bed := range beds {{\n\
             \t\tif bed > {k} && !rain {{\n\
             \t\t\ttotal += bed\n\
             \t\t}} else if bed < 0 {{\n\
             \t\t\ttotal--\n\
             \t\t}}\n\
             \t}}\n\
             \treturn total\n\
             }}\n\n"
        ),
    }
}

fn file(language: Language, f: usize) -> (String, String) {
    let (dir, extension, head) = match language {
        Language::TypeScript => ("web/beds", "ts", String::new()),
        Language::Tsx => ("web/views", "tsx", String::new()),
        Language::JavaScript => ("tools", "js", String::new()),
        Language::Python => ("scripts", "py", String::new()),
        Language::Rust => ("src/beds", "rs", String::new()),
        Language::Go => ("cmd/garden", "go", "package garden\n\n".to_owned()),
    };
    let mut text = head;
    for k in 0..FUNCTIONS {
        let _ = write!(text, "{}", function(language, f, k));
    }
    (format!("{dir}/bed_{f}.{extension}"), text)
}

fn code_lines(measurement: &Measurement) -> u64 {
    measurement
        .files
        .values()
        .map(|facts| facts.counted.lines)
        .sum()
}

fn main() -> ExitCode {
    let dir = match tempfile::tempdir() {
        Ok(dir) => dir,
        Err(err) => {
            println!("metrics: no temporary directory: {err}");
            return ExitCode::FAILURE;
        }
    };
    let path = dir.path().join("garden");
    let generated = Instant::now();
    let edits: Vec<Edit> = Language::ALL
        .into_iter()
        .flat_map(|language| (0..FILES).map(move |f| file(language, f)))
        .map(|(path, text)| Edit::write(&path, &text))
        .collect();
    let built = FixtureRepo::init(&path).and_then(|mut repo| {
        let first = repo.commit("main", Stamp::at(1_704_704_400), "plant", &edits)?;
        let (changed, text) = file(Language::Rust, 0);
        let second = repo.commit(
            "main",
            Stamp::at(1_704_790_800),
            "tend",
            &[Edit::write(
                &changed,
                &format!("{text}// TODO: one more row\n"),
            )],
        )?;
        Ok((first, second))
    });
    let (first, second) = match built {
        Ok(commits) => commits,
        Err(err) => {
            println!("metrics: the repository was not built: {err}");
            return ExitCode::FAILURE;
        }
    };
    let generated = generated.elapsed();
    let git = match GitRepo::open(&path) {
        Ok(git) => git,
        Err(err) => {
            println!("metrics: the repository did not open: {err}");
            return ExitCode::FAILURE;
        }
    };
    let never = &mut || false;
    if let Err(err) = Metrics::new().measure(&git, &first, never) {
        println!("metrics: measuring failed: {err}");
        return ExitCode::FAILURE;
    }
    let mut metrics = Metrics::new();
    let started = Instant::now();
    let cold = metrics.measure(&git, &first, never);
    let cold_took = started.elapsed();
    let started = Instant::now();
    let again = metrics.measure(&git, &second, never);
    let again_took = started.elapsed();
    let (Ok(cold), Ok(_)) = (cold, again) else {
        println!("metrics: measuring failed");
        return ExitCode::FAILURE;
    };
    let lines = code_lines(&cold);
    let files = cold.files.len();
    let parsed_again = metrics.parsed() - u64::try_from(files).unwrap_or(u64::MAX);
    if lines < LINES || parsed_again != 1 {
        println!(
            "metrics: {lines} lines of code in {files} files, {parsed_again} parsed again; \
             expected over {LINES}, and 1"
        );
        return ExitCode::FAILURE;
    }
    let within = cold_took < COLD && again_took < ONE_FILE;
    let verdict =
        |took: Duration, budget: Duration| if took < budget { "ok" } else { "OVER BUDGET" };
    println!(
        "metrics: {lines} lines of code in {files} files measured in {cold_took:.2?}, budget \
         {COLD:?}: {}; one file changed, measured again in {again_took:.2?}, budget \
         {ONE_FILE:?}: {} (generated in {generated:.2?})",
        verdict(cold_took, COLD),
        verdict(again_took, ONE_FILE),
    );
    if within {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
