#[cfg(test)]
use crate::app::state::AppState;
use crate::config;
use crate::protocol;
#[cfg(test)]
use crate::terminal::TerminalRuntimeRegistry;

pub(crate) fn should_forward_toast_to_clients(delivery: config::ToastDelivery) -> bool {
    toast_notify_kind(delivery).is_some()
}

pub(crate) fn toast_notify_kind(delivery: config::ToastDelivery) -> Option<protocol::NotifyKind> {
    match delivery {
        config::ToastDelivery::Terminal => Some(protocol::NotifyKind::Toast),
        config::ToastDelivery::System => Some(protocol::NotifyKind::SystemToast),
        config::ToastDelivery::Off | config::ToastDelivery::Herdr => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::{ToastKind, ToastNotification, ToastTarget};
    use crate::detect::{Agent, AgentState};
    use crate::terminal::TerminalState;

    fn init_repo(path: &std::path::Path) {
        let status = std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(path)
            .status()
            .unwrap();
        assert!(status.success(), "git init failed for {}", path.display());
    }

    #[tokio::test]
    async fn toast_message_uses_live_root_runtime_cwd_label() {
        let mut state = AppState::test_new();
        state
            .workspaces
            .push(crate::workspace::Workspace::test_new("stale"));
        state.ensure_test_terminals();
        let root = state.workspaces[0].tabs[0].root_pane;
        let terminal_id = state.workspaces[0].terminal_id(root).cloned().unwrap();
        let temp_root = std::env::temp_dir().join(format!(
            "herdr-forwarded-toast-context-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let stale_cwd = temp_root.join("__herdr_original__");
        let live_cwd = temp_root.join("__herdr_projects__");
        std::fs::create_dir_all(&stale_cwd).unwrap();
        std::fs::create_dir_all(&live_cwd).unwrap();
        init_repo(&stale_cwd);
        init_repo(&live_cwd);
        state.workspaces[0].custom_name = None;
        state.workspaces[0].identity_cwd = stale_cwd.clone();
        let mut terminal = TerminalState::new(terminal_id.clone(), stale_cwd);
        terminal.set_detected_state(Some(Agent::Codex), AgentState::Idle);
        state.terminals.insert(terminal_id.clone(), terminal);
        let (events, _) = tokio::sync::mpsc::channel(4);
        let runtime = crate::terminal::TerminalRuntime::spawn(
            root,
            24,
            80,
            live_cwd.clone(),
            0,
            crate::terminal_theme::TerminalTheme::default(),
            crate::pane::PaneShellConfig::new("/bin/sh", crate::config::ShellModeConfig::NonLogin),
            events,
            std::sync::Arc::new(tokio::sync::Notify::new()),
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        )
        .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while runtime.cwd() != Some(live_cwd.clone()) && std::time::Instant::now() < deadline {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let mut terminal_runtimes = TerminalRuntimeRegistry::new();
        terminal_runtimes.insert(terminal_id, runtime);

        let mut toast = ToastNotification {
            kind: ToastKind::Finished,
            title: "codex finished".to_owned(),
            context: "stale".to_owned(),
            target: Some(ToastTarget {
                workspace_id: state.workspaces[0].id.clone(),
                pane_id: root,
            }),
        };
        crate::app::notifications::refresh_notification_context(
            &state,
            &terminal_runtimes,
            &mut toast,
        );
        assert_eq!(toast.context, "__herdr_projects__ · 1");

        for (_, runtime) in terminal_runtimes.drain() {
            runtime.shutdown();
        }
        let _ = std::fs::remove_dir_all(temp_root);
    }
}
