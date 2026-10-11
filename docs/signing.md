# Signing releases

momputer's releases are signed through [SignPath Foundation](https://signpath.org), which signs open-source projects for free. The certificate belongs to SignPath Foundation, so Windows shows "SignPath Foundation" as the publisher. This page covers how it's set up and how to cut a release.

## Why it matters

The clean-ups that need an administrator run `mom.exe` elevated. If anything running as you could swap that file, your "yes" in Windows' prompt would go to whatever replaced it. So before asking, momputer checks two things (`elevate::safe_to_elevate`):

- **Where it is.** `mom.exe` and its folder can be changed only by Windows and administrators, and nothing above them is a link or could be renamed. An all-users install in Program Files passes; a copy in your own folders doesn't.
- **Who signed it.** When `momputer.exe` is signed, `mom.exe` must be signed with the very same certificate. Windows checks both signatures (offline, without asking about revocation).

As administrator, `mom` only looks, and cleans the spots that need one. Everything else it refuses, because it would act on files anything running as you can change.

## What gets signed

A release asks SignPath twice, and an approver approves each request by hand:

1. **`binaries`**: `momputer.exe` and `mom.exe`, as built by `scripts/build.ps1 -NoBundle`.
2. **`installers`**: the setup program and the MSI, built from those signed files by `scripts/build.ps1 -BundleOnly`. Building them this way leaves `momputer.exe` exactly as signed (Tauri would otherwise patch it for its updater, which momputer doesn't use). SignPath also checks that the MSI holds the signed files.

The artifact configurations are in [`.signpath/artifact-configurations`](../.signpath/artifact-configurations). SignPath only signs files that say they are momputer. The release version travels with each request, so the approver sees it.

The NSIS setup program contains an uninstaller that NSIS builds on the fly, and that uninstaller stays unsigned. It lives in Program Files next to momputer, where only administrators can change it.

## Setting it up (once)

1. **Publish a first release, unsigned.** SignPath Foundation only takes projects that are already released. Tag it (see below) and publish the draft. Without the SignPath settings, the workflow builds it unsigned and warns.
2. **Turn on two-factor sign-in** for GitHub and, later, SignPath. The Foundation requires it for everyone who can commit or approve.
3. **Apply** at [signpath.org/apply](https://signpath.org/apply). The README already has the [code signing policy](../README.md#code-signing-policy) they ask for: the attribution line, who commits and approves, and the privacy statement.
4. **Once accepted**, in SignPath:
   - Link the predefined "GitHub.com" trusted build system to the project, and install the SignPath GitHub App on this repository (both are described in [SignPath's GitHub guide](https://docs.signpath.io/trusted-build-systems/github)).
   - Create two artifact configurations, with slugs `binaries` and `installers`, from the XML files in `.signpath/artifact-configurations`.
   - Check the signing policy (usually `release-signing`): approvers are the people listed in the README, and every request needs a manual approval.
   - Create an API token for a user with the submitter role on that policy.
5. **In GitHub**, under Settings > Secrets and variables > Actions:
   - Secret `SIGNPATH_API_TOKEN`: the token.
   - Variables `SIGNPATH_ORGANIZATION_ID`, `SIGNPATH_PROJECT_SLUG` and `SIGNPATH_POLICY_SLUG`.
6. **Protect release tags.** Add a tag ruleset so only maintainers can create or move `v*` tags. A tag is what starts a signed build.

## Cutting a release

1. Bump the version in `Cargo.toml` (`[workspace.package]`) and in `app/src-tauri/tauri.conf.json`, and commit.
2. Tag and push: `git tag v0.2.0 && git push origin v0.2.0`. The workflow refuses a tag that doesn't match the version.
3. The [release workflow](../.github/workflows/release.yml) tests, builds, and asks SignPath to sign the binaries. Approve the request in SignPath within an hour. Then it builds the installers from the signed files and asks again. Approve that too.
4. A draft release appears with the signed installers and `SHA256SUMS.txt`. Check it, write a few lines about what changed, and publish.

You can also run the workflow by hand (Actions > release > Run workflow) to try it out. It drafts a release for the current version without a tag, and you can delete the draft afterwards.

## Checking a signature

```powershell
Get-AuthenticodeSignature .\momputer_0.2.0_x64-setup.exe | Format-List Status, SignerCertificate
```

`Status` should be `Valid`, with SignPath Foundation as the signer.
