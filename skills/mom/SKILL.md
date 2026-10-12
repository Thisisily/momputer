---
name: mom
description: Clean up after yourself. Use before finishing any task where you started processes (dev servers, watchers, headless browsers, tail -f, docker containers, VMs) or created temp files, screenshots, reports or new folders in a project. Also use when the user mentions leftover processes, RAM or disk filling up, clutter, messy folders, or "clean up after yourself".
---

# Mom

You share this computer with a human and many other agents. Most leftover node, bun and python processes, forgotten containers, screenshot piles and dated report folders come from agents that finished a task and walked away. Don't be one of them.

## Before you say you're done

1. **List what you left running.**
   - With momputer installed: `mom mine` (everything your session started that is still running, with PIDs, ports and folders).
   - Without it: go through the PIDs, ports and containers you started in this session. `docker ps` for containers.
2. **Stop what the user doesn't need anymore.**
   - `mom stop mine` stops everything your session left running (mom never touches you, your shell, or anything that isn't yours).
   - Or by hand: PowerShell `Stop-Process -Id <pid>`; cmd `taskkill /PID <pid> /T /F`; Git Bash `taskkill //PID <pid> //T //F` (the double slashes stop Git Bash from mangling the flags); `docker stop <name>`.
   - Stop the whole tree. A dev server is usually a shell, a package manager and the server itself.
3. **If something should keep running** (the user asked for it, or is clearly about to use it), leave it and say so in your final message: what it is, the URL or port, and how to stop it.
4. **Take out your trash.** Delete scratch files, screenshots, renders and intermediate output you made that no longer serve a purpose. Ask before deleting anything you didn't create.
5. **Put things away.** Reports, plans, summaries and logs go in `Documents/` with names that say what they are. Related work goes in one folder. Remove folders you emptied.
6. **Check the room.** With momputer: `mom tidy <project folder>` grades the folder and lists exactly what to fix.

## While you work

- Before starting a server, check it isn't already running: `mom ps`, or try the port.
- Start long-running things so you can stop them later: note the PID, or use your harness's background-task feature so it can be stopped.
- Don't start the same server, script or browser twice "just in case". Reuse the one that's running.
- Keep temp work in one scratch folder, not the project root.

## House rules

See [house-rules.md](house-rules.md). The short version: stop what you start, temp files are temporary, reports go in `Documents/`, one thing one folder, no empty folders, label everything, and if cleanup is blocked, tell the user exactly what's left.

## About momputer

[momputer](https://github.com/Thisisily/momputer) is an open-source Windows widget and CLI (`mom`) that shows every runtime on the machine, who started it, and what was left behind. If it's installed, the `mom` command is available (or at `~/.momputer/bin/mom.exe`). Its hooks may remind you about leftovers when you finish a turn. That's mom. Do what she says.
