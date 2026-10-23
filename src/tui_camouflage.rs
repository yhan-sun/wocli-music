//! Static fictional workspace: never accepts player state, machine identity or filesystem paths.
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Paragraph},
};

const SOURCE: &str = r#"use std::sync::atomic::{AtomicU64, Ordering};
use tokio::{sync::RwLock, time::timeout};
use crate::consensus::{Epoch, LogEntry, Quorum};

/// Commit a replicated epoch after quorum acknowledgement.
pub async fn commit_epoch(&self, epoch: Epoch) -> Result<Commit> {
    let fence = self.term.load(Ordering::Acquire);
    let snapshot = self.journal.read().await.snapshot();
    let batch = snapshot.compact(epoch.watermark());

    let permits = self.scheduler.reserve(batch.len()).await?;
    let replicas = self.topology.healthy_peers(epoch.region());
    let quorum = Quorum::majority(replicas.len());
    let acknowledgements = replicas.into_iter().map(|peer| {
        let payload = batch.clone();
        async move { peer.replicate(fence, payload).await }
    });

    let receipt = timeout(self.deadline, quorum.collect(acknowledgements))
        .await.map_err(Error::DeadlineExceeded)??;
    self.journal.write().await.commit(receipt.index()).await?;
    self.applied.fetch_max(receipt.index(), Ordering::AcqRel);
    self.metrics.observe_commit(epoch, receipt.latency());
    drop(permits);

    Ok(Commit::durable(fence, receipt.index()))
}

impl Scheduler {
    pub fn rebalance(&mut self, topology: &Topology) -> Plan {
        let pressure = self.queues.iter().map(Queue::pressure);
        let budget = topology.capacity().saturating_sub(self.inflight());
        let assignments = self.policy.allocate(pressure, budget);
        Plan::new(assignments).with_fencing(self.term())
    }
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn stale_epoch_cannot_commit() {
        let cluster = Cluster::deterministic(5).await;
        cluster.partition([0, 1], [2, 3, 4]).await;
        assert!(cluster.node(0).commit_epoch(Epoch::new(7)).await.is_err());
        assert_eq!(cluster.elect_leader().await.term(), 8);
    }
}"#;

pub fn draw(frame: &mut Frame, area: Rect, blink: bool) {
    let bg = Color::Rgb(18, 21, 28);
    let fg = Color::Rgb(193, 202, 218);
    let dim = Color::Rgb(100, 116, 139);
    let cyan = Color::Rgb(112, 199, 218);
    let green = Color::Rgb(140, 196, 148);
    let purple = Color::Rgb(185, 155, 223);
    frame.render_widget(Block::default().style(Style::default().bg(bg).fg(fg)), area);
    if area.height < 4 {
        frame.render_widget(
            Paragraph::new("ATLAS  /  consensus.rs").style(Style::default().fg(cyan)),
            area,
        );
        return;
    }
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(if area.height >= 22 { 7 } else { 2 }),
        Constraint::Length(1),
    ])
    .split(area);
    frame.render_widget(
        Paragraph::new(" ATLAS  /  workspace-engine                 main  +3 ~1   Rust  /  UTF-8")
            .style(Style::default().fg(fg).bg(Color::Rgb(30, 35, 46))),
        chunks[0],
    );
    frame.render_widget(
        Paragraph::new(" src / runtime / consensus.rs                 scheduler.rs   topology.rs")
            .style(Style::default().fg(dim)),
        chunks[1],
    );
    let panes = Layout::horizontal([
        Constraint::Min(1),
        Constraint::Length(if area.width >= 115 { 34 } else { 0 }),
    ])
    .split(chunks[2]);
    let lines: Vec<Line> = SOURCE
        .lines()
        .take(panes[0].height as usize)
        .enumerate()
        .map(|(i, line)| {
            let color = if line.trim_start().starts_with("//") {
                dim
            } else if line.contains("pub ")
                || line.contains("async ")
                || line.starts_with("use ")
                || line.starts_with("impl ")
            {
                purple
            } else {
                fg
            };
            Line::from(vec![
                Span::styled(format!("{:>4}  ", i + 1), Style::default().fg(dim)),
                Span::styled(line.to_string(), Style::default().fg(color)),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), panes[0]);
    if panes[1].width > 0 {
        frame.render_widget(Paragraph::new("\n  SYMBOLS\n\n  impl Consensus\n    commit_epoch()\n    apply_snapshot()\n\n  impl Scheduler\n    rebalance()\n    reserve()\n\n  DIAGNOSTICS\n\n  No issues detected\n\n  BUILD TARGET\n  x86_64 / release\n  incremental: enabled").style(Style::default().fg(dim)).block(Block::default().borders(Borders::LEFT)),panes[1]);
    }
    let prompt = if cfg!(windows) {
        "PS C:\\workspaces\\atlas> "
    } else {
        "engineer@build-node-07:/srv/atlas $ "
    };
    let logs = if area.height >= 22 {
        vec![
            Line::styled(
                " TERMINAL  /  cargo test --workspace",
                Style::default().fg(dim),
            ),
            Line::styled(
                "   Compiling workspace-engine v2.8.0",
                Style::default().fg(green),
            ),
            Line::raw("    Finished `test` profile [optimized] in 1.42s"),
            Line::raw("    Checking consensus::quorum / scheduler::fencing / storage::wal"),
            Line::styled(
                "test result: ok. 48 passed; 0 failed; finished in 0.31s",
                Style::default().fg(green),
            ),
            Line::raw(""),
            Line::from(vec![
                Span::styled(prompt, Style::default().fg(cyan)),
                Span::raw(if blink { "█" } else { " " }),
            ]),
        ]
    } else {
        vec![
            Line::styled(
                " cargo test --workspace  /  48 passed",
                Style::default().fg(green),
            ),
            Line::styled(prompt, Style::default().fg(cyan)),
        ]
    };
    frame.render_widget(
        Paragraph::new(logs).style(Style::default().fg(fg)),
        chunks[3],
    );
    frame.render_widget(
        Paragraph::new(" NORMAL   main  |  consensus.rs   |  LSP ready   |   Ln 18, Col 9")
            .style(Style::default().fg(cyan).bg(Color::Rgb(30, 35, 46))),
        chunks[4],
    );
}
