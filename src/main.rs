use ai_hive::ace::{apply, format_bullets, load, save, select_bullets, DeltaOps};
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
    /// ACE playbook curator operations
    Ace {
        #[command(subcommand)]
        cmd: AceCommands,
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
    HookStart,
    /// Stop hook
    HookStop,
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
                let hits = idx.search(&query, proj, limit, &scope)?;
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
                        let mut sections = load(&vault, &scope)?;
                        let changed = apply(&mut sections, &ops)?;
                        save(&vault, &scope, &sections, None)?;
                        println!("ace: applied {} changes to {}", changed, scope);
                    }
                    AceCommands::Show { scope, task } => {
                        let sections = load(&vault, &scope)?;
                        let t = task.as_deref().unwrap_or("");
                        let bullets = select_bullets(&sections, t, 12);
                        println!("{}", format_bullets(&bullets));
                    }
                    AceCommands::HookStart => {
                        let sections = load(&vault, "global")?;
                        let bullets = select_bullets(&sections, "", 8);
                        if !bullets.is_empty() {
                            println!("[ACE Playbook]\n{}", format_bullets(&bullets));
                        }
                    }
                    AceCommands::HookStop => {
                        println!("ace: stop hook triggered");
                    }
                }
            }
        }
    }
    Ok(())
}
