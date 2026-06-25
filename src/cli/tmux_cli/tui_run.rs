/// TUI runtime and main event loop
use anyhow::Result;
use crossterm::event::{self, Event as TermEvent, KeyCode, KeyModifiers};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io::{self, Write};
use std::time::{Duration, Instant};

use super::attach::{attach_pane_blocking, pane_for_session, session_of_pane};
use super::tui_model::{
    compute_project_tabs, fetch_tui_data, filter_live, filter_resumable, tab_project,
    update_tabs_after_refresh, PendingAttach, TuiMode,
};
use super::tui_render::{render_main, render_search};
use super::tui_terminal::TuiTerminal;
use super::verbs::selected_resume_sid;

/// TUI variant: resume `session`, returning the new pane id or an `Err(message)`
/// suitable for the status line (never writes to stderr, which raw mode mangles).
fn resume_in_tui(session: &str) -> std::result::Result<String, String> {
    let v = crate::daemon::blocking::call("tmux_resume", serde_json::json!({ "session": session }))
        .map_err(|e| format!("Resume failed: {e}"))?;
    match v["pane_id"].as_str() {
        Some(p) => Ok(p.to_string()),
        None => Err(format!(
            "Cannot resume: {}",
            v["error"].as_str().unwrap_or("unknown error")
        )),
    }
}

/// Interactive TUI for `tenex-edge tmux` (bare, no subcommand).
/// Shows live sessions and spawnable agents; lets the user attach or spawn.
pub(crate) fn tmux_tui(popup: bool) -> Result<()> {
    let refresh = Duration::from_secs(2);
    let mut selected: usize = 0;
    let mut status_msg = String::new();
    let mut tab_idx: usize = 0;
    let mut show_exited: bool = false;
    let mut exited_hours: u64 = 4;
    let mut mode = TuiMode::Normal;

    eprintln!("[tenex-edge tmux] loading sessions from daemon...");
    let _ = io::stderr().flush();
    // Initial fetch before entering raw mode: fail fast if daemon is down.
    let mut data = fetch_tui_data()?;
    eprintln!(
        "[tenex-edge tmux] loaded {} live, {} spawnable, {} resumable sessions; opening UI",
        data.live.len(),
        data.spawnable.len(),
        data.resumable.len()
    );
    let _ = io::stderr().flush();
    let mut pt = compute_project_tabs(&data);

    // Default to the project matching the current directory.
    {
        let cwd_project =
            crate::project::resolve(&std::env::current_dir().unwrap_or_default()).ok();
        if let Some(cwd_project) = cwd_project {
            if let Some(idx) = pt.visible.iter().position(|p| *p == cwd_project) {
                tab_idx = idx;
            }
        }
    }

    {
        let _terminal = TuiTerminal::enter()?;
        // Create ratatui terminal on top of the crossterm alternate screen
        // already enabled by TuiTerminal::enter().
        let mut ratatui_term = Terminal::new(CrosstermBackend::new(io::stdout()))?;

        let mut next_refresh = Instant::now() + refresh;

        loop {
            // ── draw ──────────────────────────────────────────────────────
            match &mode {
                TuiMode::Search { query, sel } => {
                    let q = query.clone();
                    let s = *sel;
                    ratatui_term.draw(|f| render_search(f, &pt, &q, s))?;
                }
                TuiMode::Normal => {
                    let exited_opt = if show_exited {
                        Some(exited_hours)
                    } else {
                        None
                    };
                    // Compute filtered totals (borrows released at end of block).
                    let total = {
                        let pf = tab_project(&pt.visible, tab_idx);
                        match pf {
                            Some(p) => {
                                filter_live(&data, p).len()
                                    + data.spawnable.len()
                                    + filter_resumable(&data, p, exited_opt).len()
                            }
                            None => 0,
                        }
                    };
                    if total > 0 && selected >= total {
                        selected = total - 1;
                    }
                    let tabs_snap = pt.visible.clone();
                    let status_snap = status_msg.clone();
                    ratatui_term.draw(|f| {
                        render_main(
                            f,
                            &data,
                            selected,
                            &status_snap,
                            &tabs_snap,
                            tab_idx,
                            exited_opt,
                        )
                    })?;
                }
            }

            let wait = next_refresh
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(100));

            let mut should_break = false;
            let mut pending_attach: Option<PendingAttach> = None;

            if event::poll(wait)? {
                if let TermEvent::Key(key) = event::read()? {
                    match &mut mode {
                        // ── search mode ───────────────────────────────────
                        TuiMode::Search { query, sel } => {
                            match key.code {
                                KeyCode::Esc => {
                                    mode = TuiMode::Normal;
                                }
                                KeyCode::Enter => {
                                    let matches = super::tui_model::fuzzy_matches(&pt, query);
                                    if let Some(proj) = matches.get(*sel).cloned() {
                                        if let Some(idx) =
                                            pt.visible.iter().position(|p| *p == proj)
                                        {
                                            tab_idx = idx;
                                        } else {
                                            // Hidden project: inject into visible temporarily.
                                            pt.hidden.retain(|p| p != &proj);
                                            pt.visible.push(proj);
                                            tab_idx = pt.visible.len() - 1;
                                        }
                                        selected = 0;
                                    }
                                    mode = TuiMode::Normal;
                                    status_msg.clear();
                                }
                                KeyCode::Up | KeyCode::Char('k') => {
                                    *sel = sel.saturating_sub(1);
                                }
                                KeyCode::Down | KeyCode::Char('j') => {
                                    let n = super::tui_model::fuzzy_matches(&pt, query).len();
                                    if *sel + 1 < n {
                                        *sel += 1;
                                    }
                                }
                                KeyCode::Backspace => {
                                    query.pop();
                                    *sel = 0;
                                }
                                KeyCode::Char(c)
                                    if !key.modifiers.contains(KeyModifiers::CONTROL) =>
                                {
                                    query.push(c);
                                    *sel = 0;
                                }
                                _ => {}
                            }
                        }
                        // ── normal mode ───────────────────────────────────
                        TuiMode::Normal => {
                            let exited_opt = if show_exited {
                                Some(exited_hours)
                            } else {
                                None
                            };
                            // We need filtered views. Use a block so borrows of
                            // `data` are released before any `data = fresh` below.
                            let total = {
                                let pf = tab_project(&pt.visible, tab_idx);
                                match pf {
                                    Some(p) => {
                                        filter_live(&data, p).len()
                                            + data.spawnable.len()
                                            + filter_resumable(&data, p, exited_opt).len()
                                    }
                                    None => 0,
                                }
                            };
                            {
                                let pf = tab_project(&pt.visible, tab_idx);
                                if pf.is_none() {
                                    continue;
                                }
                                let pf = pf.unwrap();
                                let fl = filter_live(&data, pf);
                                let fr = filter_resumable(&data, pf, exited_opt);

                                match key.code {
                                    KeyCode::Char('q') | KeyCode::Esc => {
                                        should_break = true;
                                    }
                                    KeyCode::Char('c')
                                        if key.modifiers.contains(KeyModifiers::CONTROL) =>
                                    {
                                        should_break = true;
                                    }
                                    KeyCode::Up | KeyCode::Char('k') => {
                                        selected = selected.saturating_sub(1);
                                        status_msg.clear();
                                    }
                                    KeyCode::Down | KeyCode::Char('j') => {
                                        if total > 0 && selected + 1 < total {
                                            selected += 1;
                                        }
                                        status_msg.clear();
                                    }
                                    // Left/right: switch project tabs.
                                    KeyCode::Left => {
                                        if tab_idx > 0 {
                                            tab_idx -= 1;
                                            selected = 0;
                                            status_msg.clear();
                                        }
                                    }
                                    KeyCode::Right => {
                                        if tab_idx + 1 < pt.visible.len() {
                                            tab_idx += 1;
                                            selected = 0;
                                            status_msg.clear();
                                        }
                                    }
                                    // /: enter fuzzy project search.
                                    KeyCode::Char('/') => {
                                        mode = TuiMode::Search {
                                            query: String::new(),
                                            sel: 0,
                                        };
                                    }
                                    // e: toggle exited sessions.
                                    KeyCode::Char('e') => {
                                        show_exited = !show_exited;
                                        status_msg.clear();
                                    }
                                    // +/= / -: adjust the hours window (only when exited is shown).
                                    KeyCode::Char('+') | KeyCode::Char('=') if show_exited => {
                                        exited_hours = match exited_hours {
                                            h if h >= 48 => h + 24,
                                            h if h >= 12 => h + 6,
                                            h => h + 1,
                                        };
                                        status_msg.clear();
                                    }
                                    KeyCode::Char('-') if show_exited => {
                                        exited_hours = match exited_hours {
                                            h if h > 48 => h - 24,
                                            h if h > 12 => h - 6,
                                            h => h.saturating_sub(1).max(1),
                                        };
                                        status_msg.clear();
                                    }
                                    KeyCode::Enter | KeyCode::Char('a') => {
                                        if selected < fl.len() && fl[selected].attachable {
                                            let sid = fl[selected].session_id.clone();
                                            match pane_for_session(&sid) {
                                                Some(p) => {
                                                    pending_attach = Some(PendingAttach {
                                                        pane: p,
                                                        resume_sid: Some(sid),
                                                    })
                                                }
                                                // The daemon reported the session as
                                                // attachable but has no live pane — resume
                                                // it as if it were never in tmux.
                                                None => match resume_in_tui(&sid) {
                                                    Ok(pane) => {
                                                        pending_attach = Some(PendingAttach {
                                                            pane,
                                                            resume_sid: Some(sid),
                                                        })
                                                    }
                                                    Err(msg) => status_msg = msg,
                                                },
                                            }
                                        } else {
                                            let si = selected.saturating_sub(fl.len());
                                            if selected >= fl.len() && si < data.spawnable.len() {
                                                let slug = data.spawnable[si].slug.clone();
                                                // Spawn into the selected project tab's project.
                                                let project = pf.to_string();
                                                status_msg = format!("Spawning {slug}...");
                                                // Render the status immediately before blocking.
                                                let tabs_snap = pt.visible.clone();
                                                let status_snap = status_msg.clone();
                                                let _ = ratatui_term.draw(|f| {
                                                    render_main(
                                                        f,
                                                        &data,
                                                        selected,
                                                        &status_snap,
                                                        &tabs_snap,
                                                        tab_idx,
                                                        exited_opt,
                                                    )
                                                });
                                                match crate::daemon::blocking::call(
                                                    "tmux_spawn",
                                                    serde_json::json!({
                                                        "agent": slug,
                                                        "project": project,
                                                    }),
                                                ) {
                                                    Ok(v) => {
                                                        pending_attach =
                                                            v["pane_id"].as_str().map(|p| {
                                                                PendingAttach {
                                                                    pane: p.to_string(),
                                                                    resume_sid: None,
                                                                }
                                                            });
                                                    }
                                                    Err(e) => {
                                                        status_msg = format!("Spawn failed: {e}")
                                                    }
                                                }
                                            } else {
                                                if let Some(sid) = selected_resume_sid(
                                                    &fl,
                                                    data.spawnable.len(),
                                                    &fr,
                                                    selected,
                                                ) {
                                                    status_msg = "Resuming...".to_string();
                                                    // Render the status immediately before blocking.
                                                    let tabs_snap = pt.visible.clone();
                                                    let status_snap = status_msg.clone();
                                                    let _ = ratatui_term.draw(|f| {
                                                        render_main(
                                                            f,
                                                            &data,
                                                            selected,
                                                            &status_snap,
                                                            &tabs_snap,
                                                            tab_idx,
                                                            exited_opt,
                                                        )
                                                    });
                                                    match resume_in_tui(&sid) {
                                                        Ok(pane) => {
                                                            pending_attach = Some(PendingAttach {
                                                                pane,
                                                                resume_sid: Some(sid.clone()),
                                                            })
                                                        }
                                                        Err(msg) => status_msg = msg,
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    KeyCode::Char('r') => {
                                        if let Some(sid) = selected_resume_sid(
                                            &fl,
                                            data.spawnable.len(),
                                            &fr,
                                            selected,
                                        ) {
                                            status_msg = "Resuming...".to_string();
                                            // Render the status immediately before blocking.
                                            let tabs_snap = pt.visible.clone();
                                            let status_snap = status_msg.clone();
                                            let _ = ratatui_term.draw(|f| {
                                                render_main(
                                                    f,
                                                    &data,
                                                    selected,
                                                    &status_snap,
                                                    &tabs_snap,
                                                    tab_idx,
                                                    exited_opt,
                                                )
                                            });
                                            match resume_in_tui(&sid) {
                                                Ok(pane) => {
                                                    pending_attach = Some(PendingAttach {
                                                        pane,
                                                        resume_sid: Some(sid.clone()),
                                                    })
                                                }
                                                Err(msg) => status_msg = msg,
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                                // fl, fr, pf borrows of `data` are released here.
                            }
                        }
                    }
                }
            }

            if should_break {
                break;
            }

            // Attach inline (blocking). When the user detaches (Ctrl-b d)
            // they return to this TUI.
            if let Some(pa) = pending_attach.take() {
                // Popup mode: switch the underlying tmux client to the selected
                // session and exit (closing the `display-popup`) rather than
                // nesting an attach inside the popup.
                if popup {
                    if let Some(session) = session_of_pane(&pa.pane) {
                        let _ = std::process::Command::new("tmux")
                            .args(["switch-client", "-t", &session])
                            .status();
                    }
                    break;
                }
                // Suspend ratatui/crossterm so the tmux client owns the tty.
                TuiTerminal::suspend();
                let mut res = attach_pane_blocking(&pa.pane);
                // The daemon's view of a live pane can be stale (the pane vanished
                // out from under it). A pane-not-found error must never reach the
                // user: transparently resume the session and attach to the fresh
                // pane, exactly as if it had never been in tmux.
                if res.is_err() {
                    if let Some(sid) = &pa.resume_sid {
                        if let Ok(pane) = resume_in_tui(sid) {
                            res = attach_pane_blocking(&pane);
                        }
                    }
                }
                TuiTerminal::resume();
                // ratatui needs a full redraw after the terminal is restored.
                ratatui_term.clear()?;
                status_msg = match res {
                    Ok(()) => String::new(),
                    Err(e) => format!("Attach failed: {e:#}"),
                };
                if let Ok(fresh) = fetch_tui_data() {
                    update_tabs_after_refresh(&fresh, &mut pt, &mut tab_idx);
                    data = fresh;
                }
                next_refresh = Instant::now() + refresh;
            }

            // Periodic refresh.
            if Instant::now() >= next_refresh {
                if let Ok(fresh) = fetch_tui_data() {
                    update_tabs_after_refresh(&fresh, &mut pt, &mut tab_idx);
                    data = fresh;
                }
                next_refresh = Instant::now() + refresh;
            }
        }
    }; // _terminal dropped here — raw mode disabled, alternate screen exited

    Ok(())
}
