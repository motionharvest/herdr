//! `herdr shell-init <shell>` — prints the shell integration snippet a shell
//! needs so herdr can follow its working directory.
//!
//! Only PowerShell needs this today. Linux shells expose their directory
//! through `/proc`, but a Windows PowerShell reached through WSL interop runs
//! behind a relay stub whose `/proc` entry never moves, so the shell has to
//! report its own location with OSC 7.

const POWERSHELL_SNIPPET: &str = r#"# --- herdr shell integration ---
# Reports the current location with OSC 7 so herdr can open new panes here.
if ($env:HERDR_ENV -eq '1' -and -not $global:__herdrPromptWrapped) {
    $global:__herdrPromptWrapped = $true
    $global:__herdrInnerPrompt = $function:prompt
    function global:prompt {
        $location = $ExecutionContext.SessionState.Path.CurrentLocation
        if ($location.Provider.Name -eq 'FileSystem') {
            $reported = $location.ProviderPath -replace '\\', '/'
            if ($reported.StartsWith('//')) {
                # UNC path, e.g. //wsl.localhost/Ubuntu/home/you
                $rest = $reported.Substring(2)
                $slash = $rest.IndexOf('/')
                if ($slash -ge 0) {
                    $uriHost = $rest.Substring(0, $slash)
                    $reported = $rest.Substring($slash)
                } else {
                    $uriHost = $rest
                    $reported = '/'
                }
            } else {
                $uriHost = $env:COMPUTERNAME
                if (-not $reported.StartsWith('/')) { $reported = '/' + $reported }
            }
            $encoded = ($reported.Split('/') | ForEach-Object { [uri]::EscapeDataString($_) }) -join '/'
            [Console]::Write("$([char]27)]7;file://$uriHost$encoded$([char]7)")
        }
        if ($global:__herdrInnerPrompt) { & $global:__herdrInnerPrompt }
        else { "PS $($ExecutionContext.SessionState.Path.CurrentLocation)> " }
    }
}
# --- end herdr shell integration ---
"#;

pub fn run_shell_init_command(args: &[String]) -> std::io::Result<i32> {
    match args.first().map(|arg| arg.as_str()) {
        Some("powershell" | "pwsh") if args.len() == 1 => {
            print!("{POWERSHELL_SNIPPET}");
            eprintln!("{}", powershell_instructions());
            Ok(0)
        }
        Some("help" | "--help" | "-h") => {
            print_shell_init_help();
            Ok(0)
        }
        _ => {
            print_shell_init_help();
            Ok(2)
        }
    }
}

fn powershell_instructions() -> String {
    [
        "",
        "Add the snippet above to your PowerShell profile, then start a fresh",
        "PowerShell in a herdr pane. To get it there from WSL:",
        "",
        "  herdr shell-init powershell | clip.exe",
        "",
        "then, in PowerShell, run `notepad $PROFILE` and paste it in.",
        "",
        "It only activates inside herdr panes (HERDR_ENV=1); drop that condition",
        "if you want every PowerShell session to report its location.",
    ]
    .join("\n")
}

fn print_shell_init_help() {
    eprintln!("usage: herdr shell-init <powershell>");
    eprintln!();
    eprintln!("Prints the shell integration snippet for a shell that cannot expose its");
    eprintln!("working directory to herdr on its own.");
    eprintln!();
    eprintln!("  powershell    OSC 7 location reporting for PowerShell run through WSL interop");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn powershell_snippet_guards_against_double_wrapping() {
        assert!(POWERSHELL_SNIPPET.contains("-not $global:__herdrPromptWrapped"));
        assert!(POWERSHELL_SNIPPET.contains("$global:__herdrPromptWrapped = $true"));
    }

    #[test]
    fn powershell_snippet_emits_osc7_only_for_filesystem_locations() {
        assert!(POWERSHELL_SNIPPET.contains("$location.Provider.Name -eq 'FileSystem'"));
        assert!(POWERSHELL_SNIPPET.contains("]7;file://"));
    }

    #[test]
    fn shell_init_rejects_unknown_shells() {
        assert_eq!(
            run_shell_init_command(&["fish".to_string()]).unwrap(),
            2,
            "only shells that need the snippet are accepted"
        );
        assert_eq!(run_shell_init_command(&[]).unwrap(), 2);
    }

    #[test]
    fn shell_init_accepts_powershell_spellings() {
        assert_eq!(
            run_shell_init_command(&["powershell".to_string()]).unwrap(),
            0
        );
        assert_eq!(run_shell_init_command(&["pwsh".to_string()]).unwrap(), 0);
    }
}
