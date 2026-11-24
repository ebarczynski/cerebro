//! Vault-scale benchmark for the Cerebro core library.
//!
//! Measures, on synthetic vaults of configurable size:
//!
//! * cold reindex (empty DB, N `.md` files on disk) and warm reindex (no changes)
//! * FTS5 query latency — p50 / p99 / max per query class
//! * keyword (auto) link generation
//! * graph assembly — the MCP shape (`Storage::get_graph`) and the per-note loop
//!   the Tauri `get_graph` command performs
//!
//! No benchmark framework is used (`harness = false`): every number is an exact
//! nearest-rank percentile of individual wall-clock samples, not an estimate.
//! The vault generator is seeded, so two builds of the library are always
//! measured on byte-identical input.
//!
//! ```text
//! CEREBRO_BENCH=1 cargo run --release --bench vault_bench -- --sizes 1000,5000,10000
//! CEREBRO_BENCH=1 cargo run --release --bench vault_bench -- --write-vault <dir> 5000
//! ```
//!
//! Without `CEREBRO_BENCH=1` the binary exits immediately, so
//! `cargo test --all-targets` (which executes bench binaries) stays fast.

use std::fmt::Write as _;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Serialize;

use cerebro::config::profile::ProfileConfig;
use cerebro::storage::Storage;

const SEED: u64 = 0xC3EB_B000_0000_0001;
const PER_CLASS: usize = 200;

// ── Deterministic generator ───────────────────────────────────────

/// SplitMix64 — tiny, dependency-free, reproducible across platforms.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

const FILLER: &[&str] = &[
    "the", "system", "design", "value", "record", "model", "state", "change", "group", "process",
    "control", "result", "level", "input", "output", "order", "method", "series", "table", "field",
    "review", "summary", "session", "meeting", "decision", "tradeoff", "context", "signal",
    "measure", "profile", "pattern", "boundary", "pipeline", "cluster", "service", "runtime",
    "latency", "budget", "target", "vendor", "release", "feature", "roadmap", "quarter", "owner",
    "always", "often", "never", "quickly", "later", "before", "after", "because", "although",
    "consider", "verify", "compare", "reduce", "improve", "capture", "archive", "index",
];

const TOPICS: &[&str] = &[
    "Kubernetes", "Rust", "SQLite", "Postgres", "Raft", "Consensus", "Replication", "Encryption",
    "Auth", "Cache", "Queue", "Storage", "Network", "Scheduler", "Compiler", "Testing", "Search",
    "Indexing", "Embedding", "Graph", "Parser", "Runtime", "Memory", "Profiling", "Deploy",
    "Terraform", "Ontology", "Taxonomy", "Workflow", "Journal",
];

const ADJECTIVES: &[&str] = &[
    "Practical", "Distributed", "Incremental", "Reliable", "Minimal", "Advanced", "Deferred",
    "Concurrent", "Periodic", "Offline", "Local", "Secure", "Fast", "Simple", "Robust",
];

const NOUNS: &[&str] = &[
    "Notes", "Design", "Checklist", "Deep Dive", "Failures", "Tradeoffs", "Runbook", "Summary",
    "Ideas", "Questions", "Log", "Plan", "Review", "Sketch", "Recipe",
];

/// Skewed on purpose: a handful of very common tags produces large clusters of
/// mutually "similar" notes, which is where a pairwise auto-linker hurts.
const COMMON_TAGS: &[&str] = &["note", "project/alpha", "inbox", "reading"];
const TOPIC_TAGS: &[&str] = &[
    "rust", "kubernetes", "sqlite", "distributed-systems", "databases", "security", "networking",
    "tooling", "performance", "ml", "search", "writing", "ops", "lang/rust", "lang/python",
    "kind/design", "kind/log", "kind/meeting", "status/draft", "status/done",
];

/// A synthetic note. Outgoing links are indices into the vault and are written
/// as `[[L<index>]]` placeholders, resolved to real titles by [`render`].
struct SynthNote {
    title: String,
    rel_path: String,
    tags: Vec<&'static str>,
    body: String,
}

fn generate(count: usize, seed: u64) -> Vec<SynthNote> {
    let mut rng = Rng(seed ^ (count as u64).wrapping_mul(0x100_0000_01B3));
    let mut notes = Vec::with_capacity(count);

    for i in 0..count {
        let title = format!(
            "{} {} {} {:04}",
            TOPICS[rng.below(TOPICS.len())],
            ADJECTIVES[rng.below(ADJECTIVES.len())],
            NOUNS[rng.below(NOUNS.len())],
            i % 10_000
        );
        let rel_path = format!("area-{:02}/topic-{:02}/{i:06}.md", i % 24, (i / 24) % 40);

        let mut tags: Vec<&'static str> = Vec::new();
        if rng.below(100) < 45 {
            tags.push(COMMON_TAGS[rng.below(COMMON_TAGS.len())]);
        }
        for _ in 0..(1 + rng.below(3)) {
            let tag = TOPIC_TAGS[rng.below(TOPIC_TAGS.len())];
            if !tags.contains(&tag) {
                tags.push(tag);
            }
        }

        // 3..=8 outgoing links; about one in ten dangles (to a note that never exists).
        let mut links_to: Vec<usize> = Vec::new();
        for _ in 0..(3 + rng.below(6)) {
            let target = if rng.below(100) < 10 {
                count + 1_000 + rng.below(500)
            } else {
                rng.below(count)
            };
            if target != i && !links_to.contains(&target) {
                links_to.push(target);
            }
        }

        // Roughly 300-430 words of prose.
        let sentences = 34 + rng.below(18);
        let mut body = String::with_capacity(2_800);
        let _ = writeln!(body, "# {title}\n");
        for s in 0..sentences {
            let words = 9 + rng.below(8);
            for w in 0..words {
                if w > 0 {
                    body.push(' ');
                }
                // A link in roughly every seventh sentence.
                if w == 2 && s % 7 == 3 {
                    let _ = write!(body, "[[L{}]]", links_to[(s / 7) % links_to.len()]);
                } else {
                    body.push_str(FILLER[rng.below(FILLER.len())]);
                }
            }
            body.push_str(". ");
            if s % 11 == 5 {
                let _ = write!(body, "#{} ", tags[1 % tags.len()]);
            }
        }
        // A fenced code sample holding things that must NOT become links or tags.
        let _ = write!(body, "\n```rust\nlet sample = \"[[Not A Real Link]]\"; // #not-a-tag\n```\n");

        notes.push(SynthNote {
            title,
            rel_path,
            tags,
            body,
        });
    }
    notes
}

/// Render note `i` exactly as it is written to disk.
fn render(notes: &[SynthNote], i: usize) -> String {
    let note = &notes[i];
    let mut out = String::with_capacity(note.body.len() + 160);
    let _ = write!(
        out,
        "---\ntitle: {}\ntags: [{}]\n---\n\n",
        note.title,
        note.tags.join(", ")
    );
    let mut rest = note.body.as_str();
    while let Some(pos) = rest.find("[[L") {
        out.push_str(&rest[..pos]);
        let after = &rest[pos + 3..];
        match after.find("]]") {
            Some(end) => {
                let index: usize = after[..end].parse().unwrap_or(usize::MAX);
                match notes.get(index) {
                    Some(target) => {
                        let _ = write!(out, "[[{}]]", target.title);
                    }
                    None => {
                        let _ = write!(out, "[[Missing Note {index}]]");
                    }
                }
                rest = &after[end + 2..];
            }
            None => {
                out.push_str(&rest[pos..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

fn write_vault(dir: &Path, notes: &[SynthNote]) -> io::Result<()> {
    for (i, note) in notes.iter().enumerate() {
        let path = dir.join(&note.rel_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, render(notes, i))?;
    }
    Ok(())
}

// ── Measurement helpers ────────────────────────────────────────────

/// Exact nearest-rank percentile over pre-sorted samples.
fn percentile(sorted: &[f64], p: f64) -> f64 {
    let rank = ((p / 100.0) * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

/// `(p50, p99, max)` in microseconds.
fn stats(samples: &mut [f64]) -> (f64, f64, f64) {
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    (percentile(samples, 50.0), percentile(samples, 99.0), samples[samples.len() - 1])
}

fn open_vault(profile_dir: &Path) -> Storage {
    ProfileConfig::new("bench").save_to(profile_dir).expect("write vault.toml");
    Storage::open(profile_dir, &ProfileConfig::new("bench")).expect("open storage")
}

/// Scratch profile directory; the `TempDir` must outlive every `Storage` on it.
fn temp_profile(label: &str) -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::Builder::new()
        .prefix(&format!("cerebro-bench-{label}-"))
        .tempdir()
        .expect("tempdir");
    let dir = tmp.path().join("profile");
    std::fs::create_dir_all(&dir).expect("mkdir profile");
    (tmp, dir)
}

/// A vault on disk with an empty database, ready for a cold reindex.
fn vault_on_disk(notes: &[SynthNote], label: &str) -> (tempfile::TempDir, PathBuf) {
    let holder = temp_profile(label);
    write_vault(&holder.1.join("notes"), notes).expect("write vault");
    holder
}

/// `(class, queries)` — each class stresses a different FTS5 path.
fn query_classes(notes: &[SynthNote]) -> Vec<(&'static str, Vec<String>)> {
    let n = PER_CLASS;
    let title_of = |k: usize, stride: usize| notes[(k * stride + 13) % notes.len()].title.clone();
    vec![
        ("single common word", vec!["design".to_string(); n]),
        ("single rare word", vec!["taxonomy".to_string(); n]),
        (
            "two words AND",
            (0..n)
                .map(|k| {
                    let title = title_of(k, 104_729);
                    let mut words = title.split(' ');
                    match (words.next(), words.nth(1)) {
                        (Some(a), Some(b)) => format!("{a} AND {b}"),
                        _ => "design AND system".to_string(),
                    }
                })
                .collect(),
        ),
        ("phrase", (0..n).map(|k| format!("\"{}\"", title_of(k, 7_919))).collect()),
        ("prefix query kuber*", vec!["kuber*".to_string(); n]),
        ("exact title", (0..n).map(|k| title_of(k, 7_919)).collect()),
        ("no-match term", vec!["zzzqxv".to_string(); n]),
    ]
}

struct ReindexRow {
    open: Duration,
    cold: Duration,
    warm: Duration,
    cold_changed: usize,
    warm_changed: usize,
    /// `(resolved, total)` wikilinks after the cold and after the warm pass.
    cold_links: (usize, usize),
    warm_links: (usize, usize),
    /// `db.sqlite` (+ WAL) bytes after the warm pass.
    index_bytes: u64,
}

/// Size of the database files on disk.
fn index_bytes(profile_dir: &Path) -> u64 {
    ["db.sqlite", "db.sqlite-wal"]
        .iter()
        .filter_map(|name| std::fs::metadata(profile_dir.join(name)).ok())
        .map(|meta| meta.len())
        .sum()
}

/// `(resolved, total)` wikilinks in the vault, as the MCP graph reports them.
fn resolved_links(storage: &Storage) -> (usize, usize) {
    let (_, edges) = storage.get_graph().expect("graph");
    let resolved = edges.iter().filter(|link| link.target_id.is_some()).count();
    (resolved, edges.len())
}

fn bench_reindex(notes: &[SynthNote], label: &str) -> ReindexRow {
    let (tmp, profile_dir) = vault_on_disk(notes, label);

    let t0 = Instant::now();
    let mut storage = open_vault(&profile_dir);
    let open = t0.elapsed();

    let t0 = Instant::now();
    let cold = storage.reindex_all().expect("cold reindex");
    let cold_dur = t0.elapsed();
    let cold_links = resolved_links(&storage);

    let t0 = Instant::now();
    let warm = storage.reindex_all().expect("warm reindex");
    let warm_dur = t0.elapsed();
    let warm_links = resolved_links(&storage);
    let index_bytes = index_bytes(&profile_dir);
    drop(storage);
    drop(tmp);
    ReindexRow {
        open,
        cold: cold_dur,
        warm: warm_dur,
        cold_changed: cold.created + cold.updated,
        warm_changed: warm.created + warm.updated,
        cold_links,
        warm_links,
        index_bytes,
    }
}

struct SearchRow {
    class: &'static str,
    hits: usize,
    p50: f64,
    p99: f64,
    max: f64,
}

fn bench_search(notes: &[SynthNote], label: &str) -> Vec<SearchRow> {
    let (tmp, profile_dir) = vault_on_disk(notes, label);
    let mut storage = open_vault(&profile_dir);
    storage.reindex_all().expect("reindex");

    let mut rows = Vec::new();
    for (class, queries) in query_classes(notes) {
        // Warm-up: statement cache and OS page cache.
        for query in queries.iter().take(20) {
            let _ = storage.search(query, 10);
        }
        let mut samples = Vec::with_capacity(queries.len());
        let mut hits = 0usize;
        for query in &queries {
            let t0 = Instant::now();
            hits += storage.search(query, 10).map(|r| r.len()).unwrap_or(0);
            samples.push(t0.elapsed().as_secs_f64() * 1e6);
        }
        let (p50, p99, max) = stats(&mut samples);
        rows.push(SearchRow { class, hits, p50, p99, max });
    }
    drop(storage);
    drop(tmp);
    rows
}

struct LinkGraphRow {
    keyword: Duration,
    keyword_links: usize,
    /// Bytes of JSON the Tauri `get_keyword_links` command hands to the webview.
    keyword_json_bytes: u64,
    graph_mcp: Duration,
    graph_mcp_edges: usize,
    graph_gui: Duration,
    graph_gui_edges: usize,
}

#[derive(Serialize)]
struct KeywordLink {
    source_id: String,
    target_id: String,
    confidence: f64,
}

/// Size of the JSON the GUI command produces. Serializing millions of links
/// would take gigabytes, so past a sample cap the exact size of a fixed-size
/// sample is extrapolated (every row has two fixed-width UUIDs).
fn keyword_json_bytes(links: &[(String, String, f64)]) -> u64 {
    const SAMPLE_CAP: usize = 200_000;
    let sample: Vec<KeywordLink> = links
        .iter()
        .take(SAMPLE_CAP)
        .map(|(source, target, confidence)| KeywordLink {
            source_id: source.clone(),
            target_id: target.clone(),
            confidence: *confidence,
        })
        .collect();
    let bytes = serde_json::to_string(&sample).expect("serialize").len() as u64;
    if links.len() <= SAMPLE_CAP {
        bytes
    } else {
        (bytes as f64 * links.len() as f64 / sample.len() as f64) as u64
    }
}

fn bench_links_and_graph(notes: &[SynthNote], label: &str) -> LinkGraphRow {
    let (tmp, profile_dir) = vault_on_disk(notes, label);
    let mut storage = open_vault(&profile_dir);
    storage.reindex_all().expect("reindex");

    let t0 = Instant::now();
    let links = storage.compute_keyword_links().expect("keyword links");
    let keyword = t0.elapsed();
    let keyword_links = links.len();
    let keyword_json_bytes = keyword_json_bytes(&links);
    drop(links);

    // The MCP `cerebro_graph` shape: resolved and unresolved (broken) links.
    let t0 = Instant::now();
    let (_, edges) = storage.get_graph().expect("graph");
    let graph_mcp = t0.elapsed();
    let graph_mcp_edges = edges.len();
    drop(edges);

    // Exactly the loop the Tauri `get_graph` command runs: one forward-link
    // query per note, each looked up by path.
    let t0 = Instant::now();
    let listed = storage.list_notes(None, 100_000, 0).expect("list notes");
    let mut gui_edges = 0usize;
    for note in &listed {
        if let Ok(out) = storage.get_forward_links(&note.path) {
            gui_edges += out.iter().filter(|link| link.target_id.is_some()).count();
        }
    }
    let graph_gui = t0.elapsed();

    drop(storage);
    drop(tmp);
    LinkGraphRow {
        keyword,
        keyword_links,
        keyword_json_bytes,
        graph_mcp,
        graph_mcp_edges,
        graph_gui,
        graph_gui_edges: gui_edges,
    }
}

// ── Reporting ──────────────────────────────────────────────────────

fn secs(d: Duration) -> String {
    format!("{:.3} s", d.as_secs_f64())
}

fn micros(v: f64) -> String {
    if v >= 1_000.0 {
        format!("{:.2} ms", v / 1_000.0)
    } else {
        format!("{v:.0} µs")
    }
}

struct Args {
    sizes: Vec<usize>,
    write_vault: Option<(PathBuf, usize)>,
}

fn parse_args() -> Args {
    let mut sizes = vec![1_000, 5_000];
    let mut write_vault = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--sizes" => {
                if let Some(list) = args.next() {
                    sizes = list.split(',').filter_map(|s| s.trim().parse().ok()).collect();
                }
            }
            "--write-vault" => {
                if let (Some(dir), Some(count)) = (args.next(), args.next()) {
                    write_vault = Some((PathBuf::from(dir), count.parse().unwrap_or(0)));
                }
            }
            other => panic!("unknown argument: {other}"),
        }
    }
    Args { sizes, write_vault }
}

fn main() {
    if std::env::var_os("CEREBRO_BENCH").is_none() {
        // `cargo test --all-targets` executes bench binaries; keep that fast.
        println!("vault_bench: set CEREBRO_BENCH=1 to run (sizes via --sizes 1000,5000)");
        return;
    }
    let args = parse_args();

    if let Some((dir, count)) = args.write_vault {
        let notes = generate(count, SEED);
        std::fs::create_dir_all(dir.join("notes")).expect("mkdir notes");
        write_vault(&dir.join("notes"), &notes).expect("write vault");
        println!("wrote {count} notes to {}", dir.display());
        return;
    }

    for &n in &args.sizes {
        let notes = generate(n, SEED);
        let bytes: u64 = (0..n).map(|i| render(&notes, i).len() as u64).sum();
        println!("\n━━━ {n} notes ({:.1} MB of markdown) ━━━", bytes as f64 / 1e6);

        let reindex = bench_reindex(&notes, &n.to_string());
        println!("  reindex");
        println!("    open + migrations        {:>10}", secs(reindex.open));
        println!(
            "    cold   (empty DB → full) {:>10}   {} notes written",
            secs(reindex.cold),
            reindex.cold_changed
        );
        println!(
            "    warm   (nothing changed) {:>10}   {} notes reported changed",
            secs(reindex.warm),
            reindex.warm_changed
        );
        println!(
            "    wikilinks resolved       after cold {}/{} ({:.0} %)   after warm {}/{} ({:.0} %)",
            reindex.cold_links.0,
            reindex.cold_links.1,
            100.0 * reindex.cold_links.0 as f64 / reindex.cold_links.1.max(1) as f64,
            reindex.warm_links.0,
            reindex.warm_links.1,
            100.0 * reindex.warm_links.0 as f64 / reindex.warm_links.1.max(1) as f64,
        );

        // Each benchmark is sampled exactly once; the table and the JSON line
        // below are two views of the same samples.
        let search = bench_search(&notes, &n.to_string());
        println!(
            "    index size on disk       {:>10}",
            format!("{:.1} MB", reindex.index_bytes as f64 / 1e6)
        );
        println!("  FTS5 search (limit 10, {PER_CLASS} queries per class)");
        for row in &search {
            println!(
                "    {:<22} p50 {:>10}   p99 {:>10}   max {:>10}   ({:.1} hits/query)",
                row.class,
                micros(row.p50),
                micros(row.p99),
                micros(row.max),
                row.hits as f64 / PER_CLASS as f64
            );
        }

        let lg = bench_links_and_graph(&notes, &n.to_string());
        println!("  links + graph");
        println!(
            "    keyword (auto) links     {:>10}   {} links  →  {:.1} MB of JSON for the GUI",
            secs(lg.keyword),
            lg.keyword_links,
            lg.keyword_json_bytes as f64 / 1e6
        );
        println!("    graph, MCP shape         {:>10}   {} links (resolved + broken)", secs(lg.graph_mcp), lg.graph_mcp_edges);
        println!("    graph, GUI loop          {:>10}   {} resolved edges", secs(lg.graph_gui), lg.graph_gui_edges);

        let report = serde_json::json!({
            "n": n,
            "markdown_bytes": bytes,
            "reindex_open_ms": reindex.open.as_secs_f64() * 1e3,
            "reindex_cold_ms": reindex.cold.as_secs_f64() * 1e3,
            "reindex_warm_ms": reindex.warm.as_secs_f64() * 1e3,
            "reindex_warm_changed": reindex.warm_changed,
            "cold_links_resolved": reindex.cold_links.0,
            "cold_links_total": reindex.cold_links.1,
            "warm_links_resolved": reindex.warm_links.0,
            "warm_links_total": reindex.warm_links.1,
            "index_bytes": reindex.index_bytes,
            "search": search,
            "keyword_links_ms": lg.keyword.as_secs_f64() * 1e3,
            "keyword_links": lg.keyword_links,
            "keyword_json_bytes": lg.keyword_json_bytes,
            "graph_mcp_ms": lg.graph_mcp.as_secs_f64() * 1e3,
            "graph_mcp_edges": lg.graph_mcp_edges,
            "graph_gui_ms": lg.graph_gui.as_secs_f64() * 1e3,
        });
        println!("BENCH_JSON {report}");
    }
}

impl Serialize for SearchRow {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut s = serializer.serialize_struct("SearchRow", 5)?;
        s.serialize_field("class", self.class)?;
        s.serialize_field("p50_us", &self.p50)?;
        s.serialize_field("p99_us", &self.p99)?;
        s.serialize_field("max_us", &self.max)?;
        s.serialize_field("hits", &self.hits)?;
        s.end()
    }
}
