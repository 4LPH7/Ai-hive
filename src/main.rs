use ai_hive::ace::{apply_with_lock, format_bullets, load, select_bullets, DeltaOps};
use ai_hive::bench::run_bench;
use ai_hive::common::{default_db_path, load_config};
use ai_hive::eval::run_eval;
use ai_hive::index::Index;
use ai_hive::lint::{format_report, git_staged_files, lint};
use ai_hive::mcp::{serve, tool_context, tool_graph};
use clap::{Parser, Subcommand};
use std::collections::HashSet;
use std::fs;
use std::io::{self, Read};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "hive",
    version,
    about = "High-performance shared memory for AI coding agents"
)]
struct Cli {
    #[arg(long, global = true)]
    vault: Option<PathBuf>,
    #[arg(long, global = true)]
    projects: Option<PathBuf>,
    #[arg(long, global = true)]
    db: Option<PathBuf>,
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Launch MCP stdio server (default)
    Serve {
        #[arg(long)]
        vault: Option<PathBuf>,
        #[arg(long)]
        db: Option<PathBuf>,
    },
    /// BM25 search over vault notes
    Search {
        query: String,
        project: Option<String>,
        #[arg(long, default_value = "project")]
        scope: String,
        #[arg(long, default_value = "8")]
        limit: usize,
    },
    /// Ranked code-graph neighbourhood from graphify
    Graph {
        repo: String,
        query: String,
        #[arg(long, default_value = "15")]
        limit: usize,
    },
    /// Token-budgeted memory pack for a project
    Context {
        project: String,
        task: Option<String>,
        #[arg(long, default_value = "1500")]
        budget: usize,
    },
    /// Report index and graph freshness
    Status,
    /// Vault health against AGENTS.md rules
    Lint {
        vault: Option<PathBuf>,
        #[arg(long)]
        staged: bool,
    },
    /// Run golden retrieval evaluation suite
    Eval {
        golden: Option<PathBuf>,
        #[arg(short, default_value = "5")]
        k: usize,
    },
    /// Run index, search and graph benchmarks
    Bench,
    /// Show recent session telemetry events or score session outcome
    Ledger {
        #[command(subcommand)]
        command: Option<LedgerCommands>,
        #[arg(long)]
        tail: Option<usize>,
        #[arg(long)]
        project: Option<String>,
        /// Print provenance coverage (session id + agent) instead of events
        #[arg(long)]
        coverage: bool,
    },
    /// Auto-select best skill for task, or discover and install from remote registry
    Skill {
        task: String,
        #[arg(long, default_value = "true", action = clap::ArgAction::Set)]
        auto_install: bool,
        #[arg(long, default_value = "all")]
        scope: String,
    },
    /// Run cross-platform pre-commit quality gate (lint + tests)
    PreCommit,
    /// Mine recurring zero-hit queries and skill misses into inbox proposals
    Mine {
        #[arg(long, default_value = "2")]
        min_count: usize,
        #[arg(long)]
        dry_run: bool,
    },
    /// Run OmniRoute-backed curation proposing merges and contradiction flags into inbox/
    Curate {
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        dry_run: bool,
        /// QUOR-05: judge playbook conflicts and write a cited proposal to inbox/ only
        #[arg(long)]
        conflicts: bool,
    },
    /// ACE playbook curator operations
    Ace {
        #[command(subcommand)]
        cmd: AceCommands,
    },
    /// Stigmergic traces: short decaying signals agents leave for each other (writes only hive.sqlite)
    Trace {
        #[command(subcommand)]
        cmd: TraceCommands,
    },
    /// GOV-03: provenance chain of a bullet id, trace:<n> or codex file stem
    Audit { target: String },
    /// GOV-01: reversible quarantine of everything from a session or agent (rules only; items never edited)
    Quarantine {
        #[arg(long, default_value = "")]
        session: String,
        #[arg(long, default_value = "")]
        agent: String,
        /// YYYY-MM-DD, ISO datetime or epoch seconds
        #[arg(long, default_value = "")]
        since: String,
        #[arg(long, default_value = "")]
        reason: String,
        /// Remove the rule with this id
        #[arg(long)]
        release: Option<String>,
        /// Show active rules
        #[arg(long)]
        list: bool,
    },
    /// Session codices: immutable vault notes from hive-ace checkpoint records
    Codex {
        #[command(subcommand)]
        cmd: CodexCommands,
    },
}

#[derive(Subcommand, Debug)]
pub enum CodexCommands {
    /// Write codices/<project>/YYYY-MM-DD-<sha8>.md from a JSON record file; dead ends become DEAD_END traces
    Write {
        project: String,
        record_file: PathBuf,
    },
}

#[derive(Subcommand, Debug)]
pub enum TraceCommands {
    /// Leave a trace: kind is DEAD_END, RECENT_FIX or HOT_FILE
    Add {
        project: String,
        kind: String,
        target: String,
        summary: String,
    },
    /// Show active traces for a project, strongest first
    List { project: String },
}

#[derive(Subcommand, Debug)]
pub enum LedgerCommands {
    /// Score a session outcome against test exit code and git diff
    Score {
        #[arg(long)]
        session: Option<String>,
        #[arg(long, default_value = "0")]
        exit_code: i32,
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        repo: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum AceCommands {
    /// Merge JSON delta ops into a playbook
    Apply {
        scope: String,
        ops_file: Option<PathBuf>,
    },
    /// Show bullets that hive_context would inject
    Show { scope: String, task: Option<String> },
    /// SessionStart hook
    HookStart {
        #[arg(default_value = "global")]
        scope: String,
    },
    /// Stop hook
    HookStop {
        #[arg(default_value = "global")]
        scope: String,
        #[arg(long)]
        diff: bool,
    },
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() <= 1 {
        // Default to serve when invoked with no arguments
        return serve(None, None).map_err(|e| e.into());
    } else {
        let cli = Cli::parse();
        let cfg = load_config(cli.vault, cli.projects, cli.db, cli.config.as_deref());
        match cli.command.unwrap_or(Commands::Serve {
            vault: None,
            db: None,
        }) {
            Commands::Serve { vault, db } => {
                serve(vault.or(Some(cfg.vault)), db.or(Some(cfg.db)))?;
            }
            Commands::Search {
                query,
                project,
                scope,
                limit,
            } => {
                let mut idx = Index::new(Some(cfg.vault), Some(cfg.db))?;
                let proj = project.as_deref().unwrap_or("");
                let t0 = std::time::Instant::now();
                let hits = idx.search(&query, proj, limit, &scope)?;
                let duration_ms = t0.elapsed().as_secs_f64() * 1000.0;
                let hit_paths: Vec<String> = hits.iter().map(|h| h.path.clone()).collect();
                ai_hive::ledger::record(
                    &cfg.ledger,
                    "query",
                    proj,
                    &query,
                    &hit_paths,
                    &[],
                    duration_ms,
                    None,
                    None,
                );
                if hits.is_empty() {
                    println!("no matching notes");
                } else {
                    for h in hits {
                        println!(
                            "- {} ({}, {}, {}) {}\n  {}",
                            h.path, h.r#type, h.status, h.updated, h.title, h.snippet
                        );
                    }
                }
            }
            Commands::Graph { repo, query, limit } => {
                let mut idx = Index::new(Some(cfg.vault), Some(cfg.db))?;
                let res = tool_graph(&mut idx, &repo, &query, limit)?;
                println!("{}", res);
            }
            Commands::Context {
                project,
                task,
                budget,
            } => {
                let mut idx = Index::new(Some(cfg.vault), Some(cfg.db))?;
                let t = task.as_deref().unwrap_or("");
                let res = tool_context(&mut idx, &project, t, budget)?;
                println!("{}", res);
            }
            Commands::Status => {
                let mut idx = Index::new(Some(cfg.vault), Some(cfg.db))?;
                idx.refresh(true)?;
                let total: i64 = idx
                    .conn
                    .query_row("SELECT count(*) FROM notes", [], |r| r.get(0))
                    .unwrap_or(0);
                println!(
                    "vault {}: {} notes indexed ({})",
                    idx.vault.display(),
                    total,
                    default_db_path().display()
                );
            }
            Commands::Lint { vault, staged } => {
                let v = vault.unwrap_or_else(|| cfg.vault.clone());
                let staged_set = if staged {
                    let files = git_staged_files(&v);
                    Some(files.into_iter().collect::<HashSet<String>>())
                } else {
                    None
                };
                let findings = lint(&v, staged_set.as_ref());
                println!("{}", format_report(&findings));
                if findings.iter().any(|f| f.severity == "ERROR") {
                    std::process::exit(1);
                }
            }
            Commands::Eval { golden, k } => {
                let mut idx = Index::new(Some(cfg.vault), Some(cfg.db))?;
                let golden_p = golden.unwrap_or_else(|| PathBuf::from("eval_golden.json"));
                let res = run_eval(&mut idx, &golden_p, k)?;
                println!("{}", res);
            }
            Commands::Bench => {
                let mut idx = Index::new(Some(cfg.vault), Some(cfg.db))?;
                let res = run_bench(&mut idx)?;
                println!("{}", res);
            }
            Commands::Ledger {
                command,
                tail,
                project,
                coverage,
            } => {
                if coverage && command.is_none() {
                    let entries =
                        ai_hive::ledger::read_ledger(&cfg.ledger, tail, project.as_deref());
                    let c = ai_hive::ledger::coverage(&entries);
                    let pct = |n| ai_hive::ledger::Coverage::percent(n, c.total);
                    println!(
                        "ledger coverage: {} events | session_id {:.1}% ({} process-scoped) | agent {:.1}%",
                        c.total,
                        pct(c.with_session),
                        c.fallback_session,
                        pct(c.with_agent)
                    );
                    return Ok(());
                }
                match command {
                    Some(LedgerCommands::Score {
                        session,
                        exit_code,
                        project,
                        repo,
                    }) => {
                        let res = ai_hive::ledger::score_session(
                            &cfg.ledger,
                            &cfg.vault,
                            session.as_deref(),
                            exit_code,
                            project.as_deref(),
                            repo.as_deref(),
                        )?;
                        if res.scored {
                            println!(
                                "ledger score: session={} reward={} (exit_code={}) bullets={} skills={} changes={}",
                                res.session_id, res.reward, res.test_exit_code, res.bullets.len(), res.skills.len(), res.changes
                            );
                        } else {
                            println!(
                                "ledger score: session not scored (session={})",
                                res.session_id
                            );
                        }
                    }
                    None => {
                        let entries =
                            ai_hive::ledger::read_ledger(&cfg.ledger, tail, project.as_deref());
                        if entries.is_empty() {
                            println!("ledger: no entries in {}", cfg.ledger.display());
                        } else {
                            for e in entries {
                                println!(
                                    "{} [{}] {} project={} hits={} bullets={} ({:.1}ms) - query={:?}",
                                    e.ts, e.agent, e.event, e.project, e.hits.len(), e.injected_bullets.len(), e.duration_ms, e.query
                                );
                            }
                        }
                    }
                }
            }
            Commands::Skill {
                task,
                auto_install,
                scope,
            } => {
                let res = ai_hive::mcp::tool_skill(&task, auto_install, &scope)?;
                println!("{}", res);
            }
            Commands::PreCommit => {
                println!("=== [1/2] Running hive lint ===");
                let findings = lint(&cfg.vault, None);
                println!("{}", format_report(&findings));
                if findings.iter().any(|f| f.severity == "ERROR") {
                    eprintln!("pre-commit: lint failed with errors");
                    std::process::exit(1);
                }

                println!("=== [2/2] Running Rust test suite ===");
                let status = std::process::Command::new("cargo")
                    .args(["test", "--quiet"])
                    .status();
                match status {
                    Ok(s) if s.success() => {
                        println!("All pre-commit checks passed cleanly.");
                    }
                    _ => {
                        eprintln!("pre-commit: cargo test failed");
                        std::process::exit(1);
                    }
                }
            }
            Commands::Mine { min_count, dry_run } => {
                let res = ai_hive::miner::mine_gaps(&cfg.ledger, &cfg.vault, min_count, dry_run)?;
                println!(
                    "mine: {} gaps analyzed, {} clusters, {} proposals generated (dry_run={})",
                    res.total_gaps, res.total_clusters, res.proposals_generated, res.dry_run
                );
                for p in res.proposals {
                    println!(
                        "  proposal: [{}] \"{}\" (count={}) -> {}",
                        p.hash, p.representative, p.count, p.file
                    );
                }
            }
            Commands::Curate {
                project,
                dry_run,
                conflicts: true,
            } => {
                let res = ai_hive::curate::run_curate_conflicts(
                    &cfg.vault,
                    project.as_deref(),
                    dry_run,
                    cli.config.as_deref(),
                    None,
                )?;
                if res.skipped {
                    println!(
                        "curate --conflicts: {} ({})",
                        res.model,
                        res.reason.unwrap_or_default()
                    );
                } else {
                    println!(
                        "curate --conflicts: model={} conflicts={} verdicts={}",
                        res.model, res.conflicts, res.verdicts
                    );
                    if let Some(file) = res.proposal_file {
                        let verb = if res.dry_run {
                            "would write (dry run)"
                        } else {
                            "proposal written to"
                        };
                        println!("  {verb}: {}", file);
                    }
                }
            }
            Commands::Curate {
                project, dry_run, ..
            } => {
                let res = ai_hive::curate::run_curate(
                    &cfg.ledger,
                    &cfg.vault,
                    project.as_deref(),
                    dry_run,
                    cli.config.as_deref(),
                )?;
                if res.skipped {
                    println!("curate: {} ({})", res.model, res.reason.unwrap_or_default());
                } else {
                    println!(
                        "curate: model={} merges={} contradictions={} drifts={} aliases={} (dry_run={})",
                        res.model, res.merges_count, res.contradictions_count, res.adr_drifts_count, res.alias_proposals_count, res.dry_run
                    );
                    if let Some(file) = res.proposal_file {
                        println!("  proposal written to: {}", file);
                    }
                }
            }
            Commands::Trace { cmd } => {
                if let Some(parent) = cfg.db.parent() {
                    fs::create_dir_all(parent)?;
                }
                let conn = ai_hive::index::open_db(&cfg.db)?.0; // heals a corrupt index
                conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=10000;")?;
                let now = chrono::Utc::now().timestamp();
                match cmd {
                    TraceCommands::Add {
                        project,
                        kind,
                        target,
                        summary,
                    } => {
                        let id = ai_hive::trace::add(
                            &conn,
                            &project,
                            &kind.to_ascii_uppercase(),
                            &target,
                            &summary,
                            &ai_hive::ledger::resolve_agent(None),
                            &ai_hive::ledger::resolve_session(None),
                            now,
                        )?;
                        println!(
                            "trace: {} #{} on {} ({})",
                            kind.to_ascii_uppercase(),
                            id,
                            target,
                            project
                        );
                        // GOV-04: >= 2 sessions on a DEAD_END / RECENT_FIX drafts an inbox ADD proposal
                        for p in ai_hive::trace::propose(&cfg.vault, &conn, &project, now)? {
                            println!("  proposal written to: {}", p.display());
                        }
                    }
                    TraceCommands::List { project } => {
                        let rows = ai_hive::governance::visible_traces(
                            &ai_hive::governance::load_rules(&cfg.vault),
                            ai_hive::trace::active(&conn, &project, now)?,
                        );
                        if rows.is_empty() {
                            println!("trace: no active traces for {}", project);
                        }
                        for (t, w) in rows {
                            let age_h = (now - t.created_at).max(0) / 3600;
                            println!(
                                "{:<10} {:.2}  {}  {} ({}, {}h ago)",
                                t.kind, w, t.target, t.summary, t.agent, age_h
                            );
                        }
                    }
                }
            }
            Commands::Audit { target } => {
                if let Some(parent) = cfg.db.parent() {
                    fs::create_dir_all(parent)?;
                }
                let conn = ai_hive::index::open_db(&cfg.db)?.0; // heals a corrupt index
                let lines = ai_hive::governance::audit(
                    &cfg.vault,
                    &conn,
                    &cfg.ledger,
                    &target,
                    chrono::Utc::now().timestamp(),
                )?;
                println!("{}", lines.join("\n"));
            }
            Commands::Quarantine {
                session,
                agent,
                since,
                reason,
                release,
                list,
            } => {
                use ai_hive::governance as gov;
                if list {
                    let rules = gov::load_rules(&cfg.vault);
                    if rules.is_empty() {
                        println!("quarantine: no active rules");
                    }
                    for r in rules {
                        let star = |s: &str| {
                            if s.is_empty() {
                                "*".to_string()
                            } else {
                                s.to_string()
                            }
                        };
                        println!(
                            "{} session={} agent={} since={} {}",
                            r.id,
                            star(&r.session),
                            star(&r.agent),
                            r.since,
                            r.reason
                        );
                    }
                } else if let Some(id) = release {
                    if gov::release(&cfg.vault, &id)? > 0 {
                        println!("quarantine: released {id}");
                    } else {
                        println!("quarantine: no rule {id}");
                    }
                } else {
                    if let Some(parent) = cfg.db.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    let conn = ai_hive::index::open_db(&cfg.db)?.0; // heals a corrupt index
                    let (rule, counts) = gov::quarantine(
                        &cfg.vault,
                        &conn,
                        &session,
                        &agent,
                        &since,
                        &reason,
                        chrono::Utc::now().timestamp(),
                    )?;
                    let parts: Vec<String> = gov::KINDS
                        .iter()
                        .map(|k| format!("{k}={}", counts[k]))
                        .collect();
                    println!("quarantine: {} {}", rule.id, parts.join(" "));
                }
            }
            Commands::Codex {
                cmd:
                    CodexCommands::Write {
                        project,
                        record_file,
                    },
            } => {
                let rec = ai_hive::codex::parse(&fs::read_to_string(&record_file)?)?;
                let conn = ai_hive::index::open_db(&cfg.db)?.0; // heals a corrupt index
                conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=10000;")?;
                let path = ai_hive::codex::write(
                    &cfg.vault,
                    &conn,
                    &project,
                    &rec,
                    &ai_hive::ledger::resolve_agent(None),
                    &ai_hive::ledger::resolve_session(None),
                    &chrono::Local::now().format("%Y-%m-%d").to_string(),
                    chrono::Utc::now().timestamp(),
                )?;
                println!(
                    "codex: wrote {} ({} dead-end traces)",
                    path.display(),
                    rec.dead_ends.len()
                );
                let slug = ai_hive::common::validate_slug(&project)?;
                for p in ai_hive::trace::propose(
                    &cfg.vault,
                    &conn,
                    &slug,
                    chrono::Utc::now().timestamp(),
                )? {
                    println!("  proposal written to: {}", p.display());
                }
            }
            Commands::Ace { cmd } => {
                let vault = cfg.vault;
                match cmd {
                    AceCommands::Apply { scope, ops_file } => {
                        let json_str = if let Some(path) = ops_file {
                            fs::read_to_string(path)?
                        } else {
                            let mut buf = String::new();
                            io::stdin().read_to_string(&mut buf)?;
                            buf
                        };
                        let ops: DeltaOps = ai_hive::ace::parse_delta(&json_str)?;
                        let ids = |v: &std::path::Path| -> std::collections::HashSet<String> {
                            load(v, &scope)
                                .map(|s| s.values().flatten().map(|b| b.id.clone()).collect())
                                .unwrap_or_default()
                        };
                        let before = ids(&vault);
                        let changed = apply_with_lock(&vault, &scope, &ops)?;
                        let mut added: Vec<String> =
                            ids(&vault).difference(&before).cloned().collect();
                        added.sort();
                        let content_sha = ai_hive::ace::scope_path(&vault, &scope)
                            .ok()
                            .and_then(|p| fs::read(p).ok())
                            .map(|b| ai_hive::common::sha256(&b))
                            .unwrap_or_default();
                        let cwd = std::env::current_dir().unwrap_or_default();
                        ai_hive::ledger::record_ace_apply(
                            &cfg.ledger,
                            &scope,
                            &added,
                            &content_sha,
                            &ai_hive::ledger::repo_head(&cwd),
                        );
                        println!("ace: applied {} changes to {}", changed, scope);
                    }
                    AceCommands::Show { scope, task } => {
                        let sections = ai_hive::ace::load_injectable(&vault, &scope)?;
                        let t = task.as_deref().unwrap_or("");
                        let bullets = select_bullets(&sections, t, 12);
                        println!("{}", format_bullets(&bullets));
                    }
                    AceCommands::HookStart { scope } => {
                        let sections = ai_hive::ace::load_injectable(&vault, &scope)?;
                        let bullets = select_bullets(&sections, "", 8);
                        let bullet_ids: Vec<String> =
                            bullets.iter().map(|b| b.id.clone()).collect();
                        ai_hive::ledger::record(
                            &cfg.ledger,
                            "inject",
                            &scope,
                            "hook-start",
                            &[],
                            &bullet_ids,
                            0.0,
                            None,
                            None,
                        );
                        if !bullets.is_empty() {
                            println!("[ACE Playbook ({})]\n{}", scope, format_bullets(&bullets));
                        }
                    }
                    AceCommands::HookStop { scope, diff } => {
                        ai_hive::ledger::record(
                            &cfg.ledger,
                            "reflect",
                            &scope,
                            "hook-stop",
                            &[],
                            &[],
                            0.0,
                            None,
                            None,
                        );
                        println!(
                            "ace: stop hook triggered for scope '{}' (diff={})",
                            scope, diff
                        );
                    }
                }
            }
        }
    }
    Ok(())
}
