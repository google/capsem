---
title: Shell Completions
description: Enable tab-completion for the capsem CLI in bash, zsh, fish, and PowerShell.
sidebar:
  order: 3
---

The `capsem` CLI ships with tab-completion scripts for bash, zsh, fish, and PowerShell. Generate the script once and source it from your shell's startup file.

## bash

```sh
capsem completions bash > ~/.local/share/bash-completion/completions/capsem
```

Or on macOS with Homebrew bash-completion:

```sh
capsem completions bash > /opt/homebrew/etc/bash_completion.d/capsem
```

Restart your shell (or `source` the file) and tab-completion works for `capsem <TAB>`.

## zsh

```sh
mkdir -p ~/.zfunc
capsem completions zsh > ~/.zfunc/_capsem
```

Ensure `~/.zfunc` is on your `fpath` in `~/.zshrc`:

```sh
fpath=(~/.zfunc $fpath)
autoload -Uz compinit && compinit
```

Restart your shell.

## fish

```sh
capsem completions fish > ~/.config/fish/completions/capsem.fish
```

fish picks this up automatically on the next prompt.

## PowerShell

```powershell
capsem completions powershell | Out-String | Invoke-Expression
```

To load on every session, add the line to `$PROFILE`.

## Verifying

Type `capsem ` followed by TAB. You should see subcommands (`create`, `shell`, `list`, `info`, ...). Completion is generated from the live CLI definition, so it always matches your installed version.

## Regenerating after upgrades

After `capsem update` lands a new version, regenerate the completion file. The script is static -- it doesn't call back into the binary at runtime.
