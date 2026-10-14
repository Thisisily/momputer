# momputer 0.3.1 — UI fallback

This release starts from v0.3.0 (`898565ff71cb6144e329509769e3e7a502b24d78`).
It does not include the broader backend changes from the earlier 0.3.1 candidates.

- Browser profile headers and chevrons are clickable and keyboard accessible.
- Browser hover changes the background slightly, without adding a border.
- File selection and Upkeep action state cannot silently reselect refreshed entries.
- Labels more clearly describe suggested duplicate keepers and uncertain activity.
- The main window appears in the taskbar in both compact and big modes.
- Demo asset URLs change with their contents, preventing stale UI after deployment.

File cleanup is temporarily unavailable while its safety update is validated. The UI,
native command handlers, CLI and core mutation entry points enforce this restriction.
Upkeep deletion/archive, duplicate recycling, archive restore/deletion and Room Apply/Undo
are held. Scans, previews, Room plans, process inspection and system settings remain available.

Third-party Cargo dependencies and release compiler settings are unchanged from v0.3.0.
The new backend fixes, game/application cache-owner naming, timestamp corrections,
forgotten-data disposal and pending-update cleanup safeguards are deferred. The cleanup
hold prevents using the older backend's deletion paths in this release.

An antivirus scan is an observation of the exact files and definitions tested, not proof
that every future antivirus version will accept them. A malware alert must not be bypassed.
