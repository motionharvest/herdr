use crate::api::schema::{Method, Request, ViewChange, ViewPart, ViewSetParams};

/// `herdr view <part> [show|hide|toggle] [--workspace ID] [--tab ID]`
pub(super) fn run_view_command(args: &[String]) -> std::io::Result<i32> {
    let Some(raw_part) = args.first().map(String::as_str) else {
        print_view_help();
        return Ok(2);
    };
    if matches!(raw_part, "help" | "--help" | "-h") {
        print_view_help();
        return Ok(0);
    }
    let Some(part) = parse_part(raw_part) else {
        eprintln!("unknown part: {raw_part}");
        print_view_help();
        return Ok(2);
    };

    let mut change = ViewChange::Toggle;
    let mut workspace_id = None;
    let mut tab_id = None;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "show" => change = ViewChange::Show,
            "hide" => change = ViewChange::Hide,
            "toggle" => change = ViewChange::Toggle,
            flag @ ("--workspace" | "--tab") => {
                let Some(value) = args.get(index + 1) else {
                    eprintln!("missing value for {flag}");
                    return Ok(2);
                };
                if flag == "--workspace" {
                    workspace_id = Some(super::normalize_workspace_id(value));
                } else {
                    tab_id = Some(super::normalize_tab_id(value));
                }
                index += 1;
            }
            other => {
                eprintln!("unknown option: {other}");
                return Ok(2);
            }
        }
        index += 1;
    }

    super::print_response(&super::send_request(&Request {
        id: "cli:view:set".into(),
        method: Method::ViewSet(ViewSetParams {
            part,
            change,
            workspace_id,
            tab_id,
        }),
    })?)
}

fn parse_part(raw: &str) -> Option<ViewPart> {
    Some(match raw.replace('_', "-").as_str() {
        "sidebar" => ViewPart::Sidebar,
        "spaces" => ViewPart::Spaces,
        "agent-table" => ViewPart::AgentTable,
        "agents" | "space-agents" => ViewPart::SpaceAgents,
        "group" | "space-group" => ViewPart::SpaceGroup,
        "minimap" | "minimaps" => ViewPart::Minimap,
        _ => return None,
    })
}

fn print_view_help() {
    eprintln!("herdr view commands:");
    eprintln!(
        "  herdr view <part> [show|hide|toggle] [--workspace <workspace_id>] [--tab <tab_id>]"
    );
    eprintln!();
    eprintln!("parts:");
    eprintln!("  sidebar       the whole sidebar");
    eprintln!("  spaces        the sidebar's spaces section");
    eprintln!("  agent-table   the agent table");
    eprintln!(
        "  agents        the agent rows under a space's card (--workspace, or the space on screen)"
    );
    eprintln!("  group         the worktree spaces under their repository's card (--workspace, or the space on screen)");
    eprintln!("  minimap       the layout previews under tab names (--tab, --workspace for its tabs, or every tab)");
    eprintln!();
    eprintln!("Without show or hide, the part is toggled.");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parts_parse_by_their_cli_names() {
        assert_eq!(parse_part("agent-table"), Some(ViewPart::AgentTable));
        assert_eq!(parse_part("agent_table"), Some(ViewPart::AgentTable));
        assert_eq!(parse_part("agents"), Some(ViewPart::SpaceAgents));
        assert_eq!(parse_part("minimaps"), Some(ViewPart::Minimap));
        assert_eq!(parse_part("panes"), None);
    }
}
