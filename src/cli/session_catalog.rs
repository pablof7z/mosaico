//! Scriptable local-session discovery over the operator-session projection.

mod render;

use super::interactive::session_picker::data::{fetch_sessions, SessionRow};
use crate::session_state::SessionState;
use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};

const DEFAULT_LIMIT: u64 = 20;
const MAX_LIMIT: u64 = 200;
const CURRENT_WORKSPACE_BONUS: i64 = 250;

#[derive(Subcommand)]
pub(super) enum SessionCatalogAction {
    /// List recent sessions, including stopped sessions.
    List(ListArgs),
    /// Fuzzy-search sessions across local workspaces.
    Find(FindArgs),
}

#[derive(Args)]
pub(super) struct ListArgs {
    /// Restrict results to a workspace id, name, or path.
    #[arg(long, value_name = "WORKSPACE", conflicts_with = "all_workspaces")]
    workspace: Option<String>,

    /// Include sessions from every local workspace.
    #[arg(long)]
    all_workspaces: bool,

    #[command(flatten)]
    common: CommonArgs,
}

#[derive(Args)]
pub(super) struct FindArgs {
    /// Text matched against handles, agents, work, workspaces, and runtime facts.
    query: String,

    /// Restrict results to a workspace id, name, or path.
    #[arg(long, value_name = "WORKSPACE")]
    workspace: Option<String>,

    #[command(flatten)]
    common: CommonArgs,
}

#[derive(Args)]
struct CommonArgs {
    /// Restrict results to working, idle, suspended, or offline sessions.
    #[arg(long, value_name = "STATE")]
    state: Option<String>,

    /// Show only sessions that can be resumed.
    #[arg(long)]
    resumable: bool,

    /// Show sessions active since a unix timestamp or duration such as 2h or 5d.
    #[arg(long, value_name = "WHEN")]
    since: Option<String>,

    /// Maximum rows to return.
    #[arg(
        long,
        default_value_t = DEFAULT_LIMIT,
        value_parser = clap::value_parser!(u64).range(1..=MAX_LIMIT)
    )]
    limit: u64,

    /// Number of matching rows to skip.
    #[arg(long, default_value_t = 0)]
    offset: u64,

    /// Emit stable JSON instead of human-readable text.
    #[arg(long)]
    json: bool,
}

#[derive(Clone, Copy)]
enum Mode<'a> {
    List,
    Find {
        query: &'a str,
        current_workspace: Option<&'a str>,
    },
}

pub(super) async fn run(action: SessionCatalogAction) -> Result<()> {
    let rows = fetch_sessions().await?;
    let now = crate::util::now_secs();
    let (page, json) = match action {
        SessionCatalogAction::List(args) => {
            let workspace = if args.all_workspaces {
                None
            } else {
                Some(match args.workspace {
                    Some(workspace) => workspace,
                    None => current_workspace()?,
                })
            };
            (
                query(rows, Mode::List, workspace.as_deref(), &args.common)?,
                args.common.json,
            )
        }
        SessionCatalogAction::Find(args) => {
            let query_text = args.query.trim();
            if query_text.is_empty() {
                bail!("session search query cannot be empty");
            }
            let current = current_workspace().ok();
            (
                query(
                    rows,
                    Mode::Find {
                        query: query_text,
                        current_workspace: current.as_deref(),
                    },
                    args.workspace.as_deref(),
                    &args.common,
                )?,
                args.common.json,
            )
        }
    };
    if json {
        println!("{}", render::json(&page, now)?);
    } else {
        print!("{}", render::text(&page, now));
    }
    Ok(())
}

fn current_workspace() -> Result<String> {
    let cwd = std::env::current_dir().context("reading current directory")?;
    crate::daemon::workspace_path::channel_for_path_or_bail(&cwd)
}

fn query(
    rows: Vec<SessionRow>,
    mode: Mode<'_>,
    workspace: Option<&str>,
    args: &CommonArgs,
) -> Result<Page> {
    let state = args.state.as_deref().map(parse_state).transpose()?;
    let since = args.since.as_deref().map(parse_since).transpose()?;
    let limit = usize::try_from(args.limit).context("--limit is too large")?;
    let offset = usize::try_from(args.offset).context("--offset is too large")?;
    let mut ranked = rows
        .into_iter()
        .filter(|row| workspace.is_none_or(|scope| row.matches_workspace(scope)))
        .filter(|row| state.is_none_or(|state| row.state == state))
        .filter(|row| !args.resumable || row.resumable)
        .filter(|row| since.is_none_or(|since| row.last_activity() >= since))
        .filter_map(|row| {
            let score = match mode {
                Mode::List => 0,
                Mode::Find {
                    query,
                    current_workspace,
                } => {
                    let mut score = row.fuzzy_score(query)?;
                    if current_workspace.is_some_and(|scope| row.matches_workspace(scope)) {
                        score += CURRENT_WORKSPACE_BONUS;
                    }
                    score
                }
            };
            Some((row, score))
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|(a, a_score), (b, b_score)| {
        b_score
            .cmp(a_score)
            .then_with(|| b.last_activity().cmp(&a.last_activity()))
            .then_with(|| a.handle.to_lowercase().cmp(&b.handle.to_lowercase()))
            .then_with(|| a.stable_id().cmp(&b.stable_id()))
    });
    let total = ranked.len();
    let sessions = ranked
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|(row, _)| row)
        .collect();
    Ok(Page {
        sessions,
        total,
        limit,
        offset,
        workspace: workspace.map(str::to_string),
    })
}

fn parse_state(value: &str) -> Result<SessionState> {
    SessionState::parse(value).with_context(|| {
        format!("unknown session state {value:?}; expected working, idle, suspended, or offline")
    })
}

fn parse_since(value: &str) -> Result<u64> {
    if value.parse::<u64>().is_ok() {
        return Ok(super::admin::parse_since(value));
    }
    let (number, unit) = value.split_at(value.len().saturating_sub(1));
    if !number.is_empty()
        && number.parse::<u64>().is_ok()
        && matches!(unit, "s" | "S" | "m" | "M" | "h" | "H" | "d" | "D")
    {
        return Ok(super::admin::parse_since(value));
    }
    bail!("invalid --since {value:?}; expected unix seconds or a duration such as 2h or 5d")
}

struct Page {
    sessions: Vec<SessionRow>,
    total: usize,
    limit: usize,
    offset: usize,
    workspace: Option<String>,
}

impl Page {
    fn has_more(&self) -> bool {
        self.offset.saturating_add(self.sessions.len()) < self.total
    }

    fn next_offset(&self) -> Option<usize> {
        self.has_more()
            .then(|| self.offset.saturating_add(self.sessions.len()))
    }
}

#[cfg(test)]
#[path = "session_catalog/tests.rs"]
mod tests;
