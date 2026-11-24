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
#[command(name = "hive", version = "1.2.0", about = "High-performance shared memory for AI coding agents")]
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
    /// ACE playbook curator operations
    Ace {
        #[command(subcommand)]
        cmd: AceCommands,
    },
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
    Show {
        scope: String,
        task: Option<String>,
    },
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
        match cli.command.unwrap_or(Commands::Serve { vault: None, db: None }) {
            Commands::Serve { vault, db } => {
                serve(vault.or(Some(cfg.vault)), db.or(Some(cfg.db)))?;
            }
            Commands::Search { query, project, scope, limit } => {
                let mut idx = Index::new(Some(cfg.vault), Some(cfg.db))?;
                let proj = project.as_deref().unwrap_or("");
                let t0 = std::time::Instant::now();
                let hits = idx.search(&query, proj, limit, &scope)?;
                let duration_ms = t0.elapsed().as_secs_f64() * 1000.0;
                let hit_paths: Vec<String> = hits.iter().map(|h| h.path.clone()).collect();
                ai_hive::ledger::record(&cfg.ledger, "query", proj, &query, &hit_paths, &[], duration_ms, None, None);
                if hits.is_empty() {
                    println!("no matching notes");
                } else {
                    for h in hits {
                        println!("- {} ({}, {}, {}) {}\n  {}", h.path, h.r#type, h.status, h.updated, h.title, h.snippet);
                    }
                }
            }
            Commands::Graph { repo, query, limit } => {
                let mut idx = Index::new(Some(cfg.vault), Some(cfg.db))?;
                let res = tool_graph(&mut idx, &repo, &query, limit)?;
                println!("{}", res);
            }
            Commands::Context { project, task, budget } => {
                let mut idx = Index::new(Some(cfg.vault), Some(cfg.db))?;
                let t = task.as_deref().unwrap_or("");
                let res = tool_context(&mut idx, &project, t, budget)?;
                println!("{}", res);
            }
            Commands::Status => {
                let mut idx = Index::new(Some(cfg.vault), Some(cfg.db))?;
                idx.refresh(true)?;
                let total: i64 = idx.conn.query_row("SELECT count(*) FROM notes", [], |r| r.get(0)).unwrap_or(0);
                println!("vault {}: {} notes indexed ({})", idx.vault.display(), total, default_db_path().display());
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
            Commands::Ledger { command, tail, project } => {
                match command {
                    Some(LedgerCommands::Score { session, exit_code, project, repo }) => {
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
                            println!("ledger score: session not scored (session={})", res.session_id);
                        }
                    }
                    None => {
                        let entries = ai_hive::ledger::read_ledger(&cfg.ledger, tail, project.as_deref());
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
            Commands::Skill { task, auto_install, scope } => {
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
                    .args(&["test", "--quiet"])
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
                    println!("  proposal: [{}] \"{}\" (count={}) -> {}", p.hash, p.representative, p.count, p.file);
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
                        let ops: DeltaOps = serde_json::from_str(&json_str)?;
                        let changed = apply_with_lock(&vault, &scope, &ops)?;
                        println!("ace: applied {} changes to {}", changed, scope);
                    }
                    AceCommands::Show { scope, task } => {
                        let sections = load(&vault, &scope)?;
                        let t = task.as_deref().unwrap_or("");
                        let bullets = select_bullets(&sections, t, 12);
                        println!("{}", format_bullets(&bullets));
                    }
                    AceCommands::HookStart { scope } => {
                        let sections = load(&vault, &scope)?;
                        let bullets = select_bullets(&sections, "", 8);
                        let bullet_ids: Vec<String> = bullets.iter().map(|b| b.id.clone()).collect();
                        ai_hive::ledger::record(&cfg.ledger, "inject", &scope, "hook-start", &[], &bullet_ids, 0.0, None, None);
                        if !bullets.is_empty() {
                            println!("[ACE Playbook ({})]\n{}", scope, format_bullets(&bullets));
                        }
                    }
                    AceCommands::HookStop { scope, diff } => {
                        ai_hive::ledger::record(&cfg.ledger, "reflect", &scope, "hook-stop", &[], &[], 0.0, None, None);
                        println!("ace: stop hook triggered for scope '{}' (diff={})", scope, diff);
                    }
                }
            }
        }
    }
    Ok(())
}
