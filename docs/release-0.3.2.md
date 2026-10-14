# momputer 0.3.2 — NVIDIA and Steam cache grouping

Starts from the published 0.3.1 commit `ff4f8808019e26b356feb04a556372d35c6fff0b`.

- NVIDIA shader caches group by vendor and app identity when the cache header identifies it.
- Steam shader caches show names from local game manifests. Missing or unreadable installation
  information is described cautiously; it does not prove an app was uninstalled.
- NVIDIA installer/download cache labels distinguish packages from installed drivers.
- The widget offers a separate, restricted operation for explicitly selected NVIDIA and Steam
  shader files. It uses an in-memory scan snapshot, validates file identities again, and deletes
  through the checked file handle. Newly created or changed files are left alone, as are links,
  files with alternate streams, read-only/offline files and files it cannot safely open.
- Selections are consumed after an attempt. Repeated Delete cannot select remaining files.

Shader deletion is permanent, leaves directories intact and never elevates or changes file
permissions. Games may regenerate cleared shaders and temporarily stutter. Other cleanup
remains held: browser/session data, Files duplicates, Room Apply/Undo, archives, installer
deletion, AMD/Intel/DirectX cleanup and general CLI cleanup. The prior 0.3.1 UI fixes remain.

No third-party Cargo dependency or release compiler setting changes are part of this update.
Validation results and exact installer scan hashes are recorded separately before publication.
An antivirus scan describes the tested bytes and definitions; it is not a guarantee about
future detections or a determination about earlier flagged candidates.
