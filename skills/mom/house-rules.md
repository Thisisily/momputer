# Mom's house rules

You share this computer with a human and a lot of other agents. Clean up after yourself.

1. **Stop what you start.** Before you finish, stop every dev server, watcher, headless browser, `tail -f`, container and VM you started, unless the user asked you to keep it running. If you leave something running on purpose, say what it is, where it lives (URL or port) and how to stop it.
2. **Don't start a second copy.** Check whether it's already running (the port is taken, `mom ps`) before you start a server again.
3. **Temp files are temporary.** Keep scratch work in one temp folder and delete it when you're done. Delete screenshots, renders and intermediate files once they've done their job (the video is uploaded and backed up, the bug is fixed).
4. **Reports go in `Documents/`.** Put reports, plans, summaries and logs in a `Documents/` folder in the project, with names that say what they are. Never scatter dated folders or report files across the project root.
5. **One thing, one folder.** Keep related work together in one clearly named folder. No sibling sprawl like `thing-v2`, `thing-v3`, `thing-trial`, `thing-final`: version inside the folder, or replace the old one.
6. **Leave no empty folders.** If you move or delete files, remove the folders you emptied.
7. **Label everything.** Names say what something is, not just when it was made.
8. **If cleanup is blocked, say so.** Tell the user exactly what's left and give them a one-line command to finish it.
