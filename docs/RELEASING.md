# Releasing Tandem

A runbook for publishing a new version, written so that a person or a coding agent who has
never done it can follow it start to finish. Read "How releases work" once, then use the
checklist.

## How releases work

- **A release is a Git tag.** Pushing a tag `vX.Y.Z` starts `.github/workflows/release.yml` on
  GitHub Actions. It builds the Windows installer, signs the update, and **publishes** a
  GitHub release holding three files:
  - `Tandem_X.Y.Z_x64-setup.exe`: the installer people download.
  - `Tandem_X.Y.Z_x64-setup.exe.sig`: its update signature.
  - `latest.json`: the update feed. It names the version, the installer URL, the signature and
    the release notes.
- **The workflow's steps:** checks (tag matches the version, signing key present), engine
  tests, then `tauri-apps/tauri-action`, which builds, signs, creates the release from
  `docs/release-notes.md` and uploads the three files.
- **Pushing the tag is what ships it.** Nobody clicks "Publish": when the run succeeds the
  release is public. Installed copies of Tandem read
  `https://github.com/Goose254/tandem/releases/latest/download/latest.json`, which GitHub
  serves for the latest published release, so they offer the new version on their next check.
  Treat the tag as the point of no return and do the checks before it.
- **The first public release is 1.0.2.** 1.0.0 stayed an unpublished draft and 1.0.1's run
  failed (see History), so neither was ever public.
- **Installed copies update themselves.** The app checks the feed about 5 seconds after launch
  and every 6 hours, or on demand (Settings > Updates). It offers the update only when the
  feed's version is higher than its own, verifies the signature against the public key compiled
  into the app, runs the installer and restarts. Code: `src/lib/updater.ts`,
  `src/state/updater.ts`, `src/components/updater/UpdatePrompt.tsx`.
- **Version 1.0.0 has no updater.** Anyone on a local 1.0.0 build installs a later version by
  hand once.

## The signing key

Updates are signed with a minisign key pair made by `tauri signer generate`.

| Half | Where it is | Notes |
|---|---|---|
| Public | `plugins.updater.pubkey` in `src-tauri/tauri.conf.json` | Compiled into every build. |
| Private | The maintainer's `C:\Users\<user>\.tauri\tandem-updater.key`, and the repository secret `TAURI_SIGNING_PRIVATE_KEY` | Never in the repository. Created without a password. |

Rules:

- **Never commit, print or paste the private key** anywhere except the GitHub secret field.
  An agent must not read it into its output; reading `tandem-updater.key.pub` is fine.
- **Never replace the key pair.** Installed copies only trust the public key they were built
  with. A new pair means every existing install can no longer update and must be reinstalled
  by hand. If the private key is lost, that is the only way out, so it must be backed up.
- If the key ever gets a password, also set the secret `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.
- Secrets are set by the repository owner at
  <https://github.com/Goose254/tandem/settings/secrets/actions>. An agent cannot do this step.

## Release checklist

1. **Start from a clean, green `main`.**

   ```bash
   git status
   ```

   ```bash
   npm run build
   ```

   ```bash
   cargo test --release
   ```

2. **Choose the version.** Patch (`1.0.1` to `1.0.2`) for fixes, minor (`1.1.0`) for features.
   The updater compares versions, so the new one must be higher than every published release.

3. **Set it in four files**, all to the same value. `node scripts/set-version.mjs X.Y.Z` does
   it in one go:

   | File | Field |
   |---|---|
   | `package.json` | `"version"` |
   | `src-tauri/tauri.conf.json` | `"version"` |
   | `src-tauri/Cargo.toml` | `version` under `[package]` |
   | `engine/Cargo.toml` | `version` under `[package]` |

   Then refresh the lock files, which carry the version too:

   ```bash
   npm install --package-lock-only
   ```

   ```bash
   cargo check --release
   ```

4. **Write the release notes** in `docs/release-notes.md`, replacing the previous version's.
   Plain lines or a short `-` list, most noticeable change first, in words a user would use.
   The workflow copies this file to the release page and into `latest.json`; the update card
   in the app shows its first four lines (headings and list marks stripped).

5. **If dependencies changed since the last release**, regenerate the license list shipped
   with the installer:

   ```bash
   npm run licenses
   ```

6. **Commit and push** (only when the maintainer asks; see `AGENTS.md`):

   ```bash
   git commit -am "Tandem X.Y.Z"
   ```

   ```bash
   git push origin main
   ```

   Wait for the CI run on `main` to pass: <https://github.com/Goose254/tandem/actions>.

7. **Tag and push the tag.** The tag must be `v` plus the exact version; the workflow refuses
   a mismatch.

   ```bash
   git tag vX.Y.Z
   ```

   ```bash
   git push origin vX.Y.Z
   ```

8. **Wait for the Release run** (about ten minutes). It ends with the published release at
   <https://github.com/Goose254/tandem/releases>. Without signing in, an agent can follow it
   through the public API:

   ```bash
   curl -s "https://api.github.com/repos/Goose254/tandem/actions/runs?per_page=3"
   ```

   ```bash
   curl -s https://api.github.com/repos/Goose254/tandem/releases/latest
   ```

9. **Check the release.** It must hold the three files listed above, and the feed must answer
   with the new version, the notes from step 4, and a `windows-x86_64` entry with a
   `signature`:

   ```bash
   curl -sL https://github.com/Goose254/tandem/releases/latest/download/latest.json
   ```

   Editing the release text on GitHub afterwards changes the release page but not the update
   card, which reads `latest.json` as it was built.

10. **Verify from an installed older version**: Settings > Updates > Check for updates should
    offer the new version; Update now should download, install and restart on the new version
    with settings intact. This is the only check of the whole update path (feed, signature,
    installer, restart): nothing automated covers it. Record the outcome in History.

## Building locally

| Command | What you get |
|---|---|
| `npm run release:unsigned` | The installer only. Needs no key. Use this for testing. |
| `npm run release` | The installer plus its `.sig`. Needs the private key (below). |

For a signed local build, point the variable at the key file (a path is accepted) in the same
shell:

```bash
export TAURI_SIGNING_PRIVATE_KEY="$HOME/.tauri/tandem-updater.key" TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""
```

```bash
npm run release
```

Output: `target/release/bundle/nsis/`.

**If Tandem is running from `target/release/tandem.exe`, the build fails with "Access is
denied"** because Windows locks a running program. Do not close the maintainer's app. Rename
the file instead (Windows allows renaming a running program) and build again; delete the
renamed file once the app has been restarted:

```bash
mv target/release/tandem.exe target/release/tandem.in-use.exe
```

## When something goes wrong

| Symptom | Cause | Fix |
|---|---|---|
| Release run fails at "Check that the tag matches the app version" | The tag and `src-tauri/tauri.conf.json` disagree | Fix the version, commit, delete the tag locally and on GitHub, tag again |
| Release run fails at "Check that the update signing key is set" | The `TAURI_SIGNING_PRIVATE_KEY` secret is missing | The owner adds it (link above), then re-run the job |
| Build fails with "A public key has been found, but no private key" | A signed build without the key | Set the variable, or use `npm run release:unsigned` |
| Build fails with a signature or password error | Wrong key, or a password-protected key without its password | Use the key that matches `pubkey`; set the password secret |
| The app says "You have the latest version" after a release | The run has not finished, the release is a draft or a pre-release, or its version is not higher | Check the run and the version in `latest.json` |
| "Couldn't check for updates" in the app | The latest published release has no `latest.json`, or no network | Make sure the latest release was built by this workflow |
| A bad release is already public | Tags publish directly | Fix it and release a higher version; installed copies only move forward. Deleting the release on GitHub (owner only) stops new installs meanwhile |
| Release run fails in `tauri-action` with "Resource not accessible by integration" | Seen once, when the workflow asked for a draft release | Keep `releaseDraft: false`; check the workflow still has `permissions: contents: write` |
| Windows SmartScreen warns on the installer | The installer is not code-signed (update signing is a different thing) | Expected; "More info" then "Run anyway". Code signing needs a paid certificate |

To redo a tag whose run failed before it published anything:

```bash
git tag -d vX.Y.Z
```

```bash
git push origin :refs/tags/vX.Y.Z
```

Fix what was wrong and tag again. If the run got as far as creating the release, the owner
deletes that release on GitHub first; otherwise use a higher version instead.

## History

| Version | What |
|---|---|
| 1.0.0 | First build. Tagged; its release stayed a draft and was never published. No updater. |
| 1.0.1 | Twenty audit fixes, the three-note icon, in-app updates. Tagged, but its run failed at the last step and created no release: with `releaseDraft: true`, GitHub refused the create-release call ("Resource not accessible by integration"). |
| 1.0.2 | Same code as 1.0.1 plus the documentation. First published release, and the first built by the workflow that publishes on its own. |
| 1.0.3 | The boxes grow with the window; the update card's buttons fit and the card is never see-through. First release that installed copies (1.0.2) are offered as an update. That update has not yet been confirmed end to end (checklist step 10): note the result here once it has been run. |
| 1.1.0 | "Line up with" keeps its sweeps off the source, other programs are silenced for a measurement started by hand, a drawing for every auto-sync state, a first-run guide. Released on the maintainer's go-ahead after trying the builds; the hardware checklist's steps 11 to 18 and 24 to 28 were not recorded as run one by one (see "Known gaps" in `AGENTS.md`). |
