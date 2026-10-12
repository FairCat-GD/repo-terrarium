//! Source metrics: complexity, TODO density, lines of code and exported items, parsed in
//! process with tree-sitter from the blobs one commit holds. The worktree is never read, and
//! nothing runs (I-5).
//!
//! Only blobs not met before are parsed: a file's facts are kept by its blob and language, so a
//! commit that changed three files costs three parses. Kept facts last while a measurement or
//! the one before it used them, and live in memory only.
//!
//! A file is read in full or not at all. Over [`MAX_SOURCE_BYTES`] it is skipped; a parse that
//! runs past [`PARSE_LIMIT`] counts nothing; one with syntax errors still counts what it holds,
//! and the readings' confidence falls with each. A blob that cannot be read costs that file
//! alone.

mod count;
mod exports;
mod lang;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, HashMap};
use std::ops::ControlFlow;
use std::time::{Duration, Instant};

use terrarium_truth::{CommitId, Q16};
use tree_sitter::{ParseOptions, ParseState, Parser, Point};

pub use count::Counted;
pub use lang::Language;

use crate::git::{Blob, GitError, GitRepo};
use crate::report::{Contributor, KnipCounts, ratio, top};

/// The `adapter_version` of the complexity and TODO density these metrics give. It changes when
/// the same commit would be counted differently.
pub const METRICS_VERSION: u32 = 1;
/// A source file larger than this is not read: at that size it is generated or bundled.
pub const MAX_SOURCE_BYTES: u64 = 1024 * 1024;
/// The longest one file's parse may take before it counts nothing.
pub const PARSE_LIMIT: Duration = Duration::from_secs(2);
/// Complexity is 0 at a mean cyclomatic complexity of 15 (`(mean − 1) / 14`).
const COMPLEXITY_SPAN: u64 = 14;
/// TODO density is 0 at 20 markers per 1,000 lines: one marker per 50 lines.
const LINES_PER_MARKER_AT_ZERO: u64 = 50;

/// How a source file was read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Read {
    Clean,
    /// Parsed with syntax errors: what it holds still counts.
    Partial,
    /// Counts nothing: its parse ran past [`PARSE_LIMIT`], or its blob could not be read.
    Unparsed,
    /// Over [`MAX_SOURCE_BYTES`], so never read.
    Oversized,
}

impl Read {
    const fn counted(self) -> bool {
        matches!(self, Read::Clean | Read::Partial)
    }
}

/// What one source file holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileFacts {
    pub language: Language,
    pub read: Read,
    pub counted: Counted,
    /// Distinct exported names; counted for TypeScript, TSX and JavaScript only.
    pub exports: u64,
}

impl FileFacts {
    const fn unread(language: Language, read: Read) -> FileFacts {
        FileFacts {
            language,
            read,
            counted: Counted {
                lines: 0,
                functions: 0,
                decision_points: 0,
                max_cyclomatic: 0,
                max_nesting: 0,
                nesting_total: 0,
                todo: 0,
                fixme: 0,
                hack: 0,
            },
            exports: 0,
        }
    }
}

/// How many source files a reading stands on, and how they were read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sources {
    pub files: u64,
    pub partial: u64,
    pub unparsed: u64,
    pub oversized: u64,
}

impl Sources {
    /// The share of the files read that parsed without an error; 1 when none was read.
    pub fn confidence(&self) -> Q16 {
        let clean = self.files - self.oversized - self.partial - self.unparsed;
        let read = self.files - self.oversized;
        if read == 0 {
            return Q16::ONE;
        }
        ratio(clean, read)
    }
}

/// Complexity, with what it was computed from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComplexityReading {
    pub sources: Sources,
    pub functions: u64,
    pub decision_points: u64,
    pub max_cyclomatic: u64,
    pub max_nesting: u64,
    pub nesting_total: u64,
    /// `1 − clamp((mean_cyclomatic − 1) / 14)`.
    pub normalized: Q16,
    /// The files with the most decisions inside functions.
    pub contributors: Vec<Contributor>,
}

/// TODO density, with what it was computed from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TodoReading {
    pub sources: Sources,
    pub lines: u64,
    pub todo: u64,
    pub fixme: u64,
    pub hack: u64,
    /// `1 − clamp(todos_per_kloc / 20)`.
    pub normalized: Q16,
    /// The files with the most markers.
    pub contributors: Vec<Contributor>,
}

/// Dead code: a Knip report's unused items against the exports counted here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeadCodeReading {
    pub unused_items: u32,
    pub exported_items: u32,
    /// `1 − min(1, unused_items / max(1, exported_items))`.
    pub normalized: Q16,
    pub confidence: Q16,
    /// The files with the most unused items.
    pub contributors: Vec<Contributor>,
}

/// The source files of one commit, each with its facts, by path.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Measurement {
    pub files: BTreeMap<String, FileFacts>,
}

impl Measurement {
    /// Whether the commit holds any source file in a language read here.
    pub fn has_source(&self) -> bool {
        !self.files.is_empty()
    }

    fn sources(&self, of: impl Fn(Language) -> bool) -> Sources {
        let mut sources = Sources::default();
        for facts in self.files.values().filter(|facts| of(facts.language)) {
            sources.files += 1;
            match facts.read {
                Read::Clean => {}
                Read::Partial => sources.partial += 1,
                Read::Unparsed => sources.unparsed += 1,
                Read::Oversized => sources.oversized += 1,
            }
        }
        sources
    }

    fn counted(&self) -> impl Iterator<Item = (&String, &FileFacts)> {
        self.files.iter().filter(|(_, facts)| facts.read.counted())
    }

    /// Complexity, unless no function was found.
    pub fn complexity(&self) -> Option<ComplexityReading> {
        let mut reading = ComplexityReading {
            sources: self.sources(|_| true),
            functions: 0,
            decision_points: 0,
            max_cyclomatic: 0,
            max_nesting: 0,
            nesting_total: 0,
            normalized: Q16::ZERO,
            contributors: Vec::new(),
        };
        let mut weights = Vec::new();
        for (path, facts) in self.counted() {
            let c = &facts.counted;
            reading.functions += c.functions;
            reading.decision_points += c.decision_points;
            reading.max_cyclomatic = reading.max_cyclomatic.max(c.max_cyclomatic);
            reading.max_nesting = reading.max_nesting.max(c.max_nesting);
            reading.nesting_total += c.nesting_total;
            weights.push(contributor(path, c.decision_points));
        }
        if reading.functions == 0 {
            return None;
        }
        reading.normalized = Q16::ONE.saturating_sub(ratio(
            reading.decision_points,
            reading.functions.saturating_mul(COMPLEXITY_SPAN),
        ));
        reading.contributors = top(weights);
        Some(reading)
    }

    /// TODO density, unless no line of code was found.
    pub fn todo_density(&self) -> Option<TodoReading> {
        let mut reading = TodoReading {
            sources: self.sources(|_| true),
            lines: 0,
            todo: 0,
            fixme: 0,
            hack: 0,
            normalized: Q16::ZERO,
            contributors: Vec::new(),
        };
        let mut weights = Vec::new();
        for (path, facts) in self.counted() {
            let c = &facts.counted;
            reading.lines += c.lines;
            reading.todo += c.todo;
            reading.fixme += c.fixme;
            reading.hack += c.hack;
            weights.push(contributor(path, c.todo + c.fixme + c.hack));
        }
        if reading.lines == 0 {
            return None;
        }
        let markers = reading.todo + reading.fixme + reading.hack;
        reading.normalized = Q16::ONE.saturating_sub(ratio(
            markers.saturating_mul(LINES_PER_MARKER_AT_ZERO),
            reading.lines,
        ));
        reading.contributors = top(weights);
        Some(reading)
    }

    /// Dead code from `knip`'s report, read against the exports counted here: Knip's unused
    /// exports and types, plus the exports of the files it finds unused, in the files parsed
    /// here alone, so a file this reader does not parse (a `.vue` one, say) weighs on neither
    /// side. Unknown when no export was counted (I-3: a count of 0 is no reading).
    pub fn dead_code(&self, knip: &KnipCounts) -> Option<DeadCodeReading> {
        let parsed = |path: &str| {
            self.files
                .get(path)
                .filter(|facts| facts.language.exports_counted() && facts.read.counted())
        };
        let exported: u64 = self
            .counted()
            .filter(|(_, facts)| facts.language.exports_counted())
            .map(|(_, facts)| facts.exports)
            .sum();
        if exported == 0 {
            return None;
        }
        let mut unused: BTreeMap<&str, u64> = BTreeMap::new();
        for (path, count) in &knip.unused_exports_per_file {
            if parsed(path).is_some() {
                *unused.entry(path).or_default() += count;
            }
        }
        for path in &knip.unused_files {
            if let Some(facts) = parsed(path) {
                *unused.entry(path).or_default() += facts.exports;
            }
        }
        let unused_items: u64 = unused.values().sum();
        let saturated = |n: u64| u32::try_from(n).unwrap_or(u32::MAX);
        Some(DeadCodeReading {
            unused_items: saturated(unused_items),
            exported_items: saturated(exported),
            normalized: Q16::ONE.saturating_sub(ratio(unused_items, exported)),
            confidence: self.sources(Language::exports_counted).confidence(),
            contributors: top(unused
                .into_iter()
                .map(|(path, count)| contributor(path, count))
                .collect()),
        })
    }
}

fn contributor(path: &str, weight: u64) -> Contributor {
    Contributor {
        path: path.to_owned(),
        value: i64::try_from(weight).unwrap_or(i64::MAX),
    }
}

/// The files of each language `commit` holds, by path alone: nothing is parsed.
pub fn languages_at(git: &GitRepo, commit: &CommitId) -> Result<BTreeMap<Language, u64>, GitError> {
    let mut languages = BTreeMap::new();
    for file in git.files_at(commit)? {
        if let Some(language) = Language::of_path(&file.path) {
            *languages.entry(language).or_default() += 1;
        }
    }
    Ok(languages)
}

/// A grammar loaded into its parser, with its nodes' roles.
struct Grammar {
    parser: Parser,
    roles: count::Roles,
}

impl Grammar {
    /// `None` when the grammar's ABI is one tree-sitter cannot load (the table test checks
    /// every one); its files then count as unparsed.
    fn load(language: Language) -> Option<Grammar> {
        let grammar = language.grammar();
        let mut parser = Parser::new();
        parser.set_language(&grammar).ok()?;
        Some(Grammar {
            parser,
            roles: count::Roles::new(&grammar, language.table()),
        })
    }
}

/// A file's facts, and the last measurement that used them.
struct Kept {
    /// `None` for a binary blob.
    facts: Option<FileFacts>,
    used: u64,
}

/// The source metrics' parsers and the facts of every file they have read.
pub struct Metrics {
    grammars: BTreeMap<Language, Option<Grammar>>,
    kept: HashMap<(gix::ObjectId, Language), Kept>,
    generation: u64,
    parsed: u64,
    limit: Duration,
}

impl Default for Metrics {
    fn default() -> Self {
        Metrics::new()
    }
}

impl Metrics {
    pub fn new() -> Metrics {
        Metrics::with_limit(PARSE_LIMIT)
    }

    /// With a parse limit of its own, for tests.
    pub(crate) fn with_limit(limit: Duration) -> Metrics {
        Metrics {
            grammars: BTreeMap::new(),
            kept: HashMap::new(),
            generation: 0,
            parsed: 0,
            limit,
        }
    }

    /// How many files have been parsed, all measurements together.
    pub fn parsed(&self) -> u64 {
        self.parsed
    }

    /// The source files of `commit`, parsing only those not met before. `stop` is asked
    /// between files and during a parse; when it says so, the measurement ends with
    /// [`GitError::Cancelled`] and the facts read so far are kept.
    pub fn measure(
        &mut self,
        git: &GitRepo,
        commit: &CommitId,
        stop: &mut dyn FnMut() -> bool,
    ) -> Result<Measurement, GitError> {
        let files = git.files_at(commit)?;
        self.generation += 1;
        let generation = self.generation;
        let mut measurement = Measurement::default();
        for file in files {
            let Some(language) = Language::of_path(&file.path) else {
                continue;
            };
            if stop() {
                return Err(GitError::Cancelled);
            }
            let key = (file.id, language);
            let facts = if let Some(kept) = self.kept.get_mut(&key) {
                kept.used = generation;
                kept.facts
            } else {
                let (facts, keep) = match git.blob(file.id, MAX_SOURCE_BYTES) {
                    Blob::Binary => (None, true),
                    Blob::Oversized => (Some(FileFacts::unread(language, Read::Oversized)), true),
                    // It may be readable later (a partial clone fetches it).
                    Blob::Unreadable => (Some(FileFacts::unread(language, Read::Unparsed)), false),
                    Blob::Text(source) => (Some(self.parse(language, &source, stop)?), true),
                };
                if keep {
                    self.kept.insert(
                        key,
                        Kept {
                            facts,
                            used: generation,
                        },
                    );
                }
                facts
            };
            if let Some(facts) = facts {
                measurement
                    .files
                    .insert(String::from_utf8_lossy(&file.path).into_owned(), facts);
            }
        }
        self.kept.retain(|_, kept| kept.used + 1 >= generation);
        Ok(measurement)
    }

    /// One file's facts. A parse stopped by the limit counts nothing; one stopped by `stop`
    /// cancels the measurement. Either way the parser is reset, so the next file starts afresh.
    fn parse(
        &mut self,
        language: Language,
        source: &[u8],
        stop: &mut dyn FnMut() -> bool,
    ) -> Result<FileFacts, GitError> {
        let Some(grammar) = self
            .grammars
            .entry(language)
            .or_insert_with(|| Grammar::load(language))
        else {
            return Ok(FileFacts::unread(language, Read::Unparsed));
        };
        self.parsed += 1;
        let deadline = Instant::now() + self.limit;
        let mut cancelled = false;
        let mut progress = |_: &ParseState| {
            if stop() {
                cancelled = true;
                ControlFlow::Break(())
            } else if Instant::now() >= deadline {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        };
        let mut input = |offset: usize, _: Point| source.get(offset..).unwrap_or(&[]);
        let tree = grammar.parser.parse_with_options(
            &mut input,
            None,
            Some(ParseOptions::new().progress_callback(&mut progress)),
        );
        let Some(tree) = tree else {
            grammar.parser.reset();
            return if cancelled {
                Err(GitError::Cancelled)
            } else {
                Ok(FileFacts::unread(language, Read::Unparsed))
            };
        };
        let read = if tree.root_node().has_error() {
            Read::Partial
        } else {
            Read::Clean
        };
        Ok(FileFacts {
            language,
            read,
            counted: count::count(&tree, source, &grammar.roles),
            exports: if language.exports_counted() {
                exports::count(&tree, source)
            } else {
                0
            },
        })
    }
}
