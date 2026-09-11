//! The command groups `capsem --help` prints, maintained beside the enums
//! they describe.

pub(crate) const GROUPED_HELP: &str = "\
\x1b[36;1;4mSession Commands:\x1b[0m
  \x1b[32;1mcreate\x1b[0m       Create and boot a new session
  \x1b[32;1mshell\x1b[0m        Open an interactive shell in a session
  \x1b[32;1mresume\x1b[0m       Resume a suspended session or attach to a running one
  \x1b[32;1msuspend\x1b[0m      Suspend a running session to disk
  \x1b[32;1mrestart\x1b[0m      Restart a persistent session (reboot)
  \x1b[32;1mexec\x1b[0m         Execute a command in a running session
  \x1b[32;1mrun\x1b[0m          Run a command in a fresh session (destroyed after)
  \x1b[32;1mlist\x1b[0m         List all sessions (running + suspended persistent)
  \x1b[32;1minfo\x1b[0m         Show detailed information about a session
  \x1b[32;1mlogs\x1b[0m         Show logs from a session
  \x1b[32;1mdelete\x1b[0m       Delete a session and all its state
  \x1b[32;1mfork\x1b[0m         Fork a session into a reusable snapshot
  \x1b[32;1mpersist\x1b[0m      Promote an ephemeral session to persistent
  \x1b[32;1mpurge\x1b[0m        Destroy all temporary sessions

\x1b[36;1;4mService:\x1b[0m
  \x1b[32;1minstall\x1b[0m      Install as a system service (LaunchAgent / systemd)
  \x1b[32;1mstatus\x1b[0m       Show service status
  \x1b[32;1mstart\x1b[0m        Start the background service
  \x1b[32;1mstop\x1b[0m         Stop the background service
  \x1b[32;1massets\x1b[0m       Inspect or repair VM assets

\x1b[36;1;4mNetworks:\x1b[0m
  \x1b[32;1mnetwork list\x1b[0m        List named networks
  \x1b[32;1mnetwork create\x1b[0m      Create a named network
  \x1b[32;1mnetwork inspect\x1b[0m     Show a network's id and members
  \x1b[32;1mnetwork delete\x1b[0m      Retire an empty network
  \x1b[32;1mnetwork connect\x1b[0m     Connect a session to a network
  \x1b[32;1mnetwork disconnect\x1b[0m  Disconnect a session from a network
  \x1b[32;1mnetwork logs\x1b[0m        Show a network's audit log (-f to follow)

\x1b[36;1;4mMCP:\x1b[0m
  \x1b[32;1mmcp servers\x1b[0m  List configured MCP servers with connection status
  \x1b[32;1mmcp tools\x1b[0m    List discovered MCP tools across all servers
  \x1b[32;1mmcp refresh\x1b[0m  Re-discover tools from all MCP servers
  \x1b[32;1mmcp call\x1b[0m     Call an MCP tool

\x1b[36;1;4mMisc:\x1b[0m
  \x1b[32;1mupdate\x1b[0m       Check for updates and install the latest version
  \x1b[32;1mdoctor\x1b[0m       Run diagnostic tests in a fresh session
  \x1b[32;1mdebug\x1b[0m        Write a redacted support bundle for bug reports
  \x1b[32;1mcompletions\x1b[0m  Generate shell completions (bash, zsh, fish, powershell)
  \x1b[32;1mversion\x1b[0m      Show version and build information
  \x1b[32;1muninstall\x1b[0m    Uninstall capsem completely (service, binaries, data)";
