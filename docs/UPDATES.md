# Updates

bitBybit updates itself from the GitHub Releases of this repository (like OrbisLink). No server of our own, no
account, no telemetry.

## Channels

| Channel | What it gets | Where it comes from |
|---|---|---|
| `testing` (default while bitBybit is a preview) | every published build | a GitHub **pre-release** `vX.Y.Z-dev.N`, published by CI for every push to the development branch (N = workflow run number) |
| `stable` | stable versions only | a GitHub release `vX.Y.Z`, published by CI when the owner pushes the tag `vX.Y.Z` (must equal `VersionPrefix`) |

Semver ordering: `0.5.0-dev.12 < 0.5.0 < 0.5.1-dev.1`. A testing user gets the stable when it is published; a stable user
is never moved to a testing build. The app never installs an older or equal version (downgrade protection).

## Trust chain (fail closed)

1. CI builds `bitBybit.exe` and `bitbybit-cli.exe` with `-p:Version=<version>`.
2. `bitbybit-release manifest` writes `bitbybit-manifest.json` (version, channel, commit, SHA-256 and size of each file).
3. `bitbybit-release sign` signs the manifest bytes with ECDSA P-256 / SHA-256 (IEEE P1363, base64 in
   `bitbybit-manifest.json.sig`). The private key comes only from the Actions secret `BITBYBIT_UPDATE_SIGNING_KEY`.
   Without the secret nothing is published.
4. `bitbybit-release verify` checks the signature with the public key(s) **embedded in the app**
   (`src/BitByBit.Core/Updates/TrustedUpdateKeys/*.pub.pem`) and every file's hash — CI refuses to publish what the app
   would reject.
5. The app (`UpdateService`) lists releases (`api.github.com/repos/<repo>/releases`), picks the newest of its channel that
   is newer than itself and has manifest + signature + exe, downloads only from
   `https://github.com/<repo>/releases/download/…`, verifies the signature with its embedded key, checks that the manifest
   version equals the release tag and is newer than the installed one, streams `bitBybit.exe` while hashing it and
   compares size and SHA-256. Any failure: the update is refused.
6. Swap: the running `bitBybit.exe` is renamed to `bitBybit.exe.old-<version>` (Windows allows renaming a running exe),
   the verified file is copied in its place (on failure the original is put back), the new version starts with
   `--after-update` and removes the old file and the staging folder.

## Keys

* Generate: `bitbybit-release keygen --private <file outside the repo> --public src/BitByBit.Core/Updates/TrustedUpdateKeys/<name>.pub.pem`.
* The private key goes only into the repository secret `BITBYBIT_UPDATE_SIGNING_KEY` (Settings › Secrets and variables ›
  Actions). `*.pem` files are git-ignored except `TrustedUpdateKeys/*.pub.pem`.
* Rotation: add the new public key next to the old one, ship a build, then switch the secret; remove the old public key in
  a later build.
