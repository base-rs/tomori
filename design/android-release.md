# Android Release

This describes how Tomori's headset APK is released, and how its release key is kept.

## Requirement

Each release of Tomori attaches its headset APK to the release's GitHub Release, as `tomori-VERSION.apk`, with `tomori-VERSION.apk.sha256` beside it. The APK is Tomori's own build, `com.codeotaku.tomori`, in the release profile, built from the release's commit, and is not debuggable. `releases.md`'s notes are the Release's, as for the crates.

It is signed with Tomori's release key, which identifies the app for every later update:

- The key is a PKCS12 keystore holding one key, whose key password is the keystore's. The maintainer generates it, keeps its only backup offline, and adds it, base64-encoded, and its password as the secrets `ANDROID_RELEASE_KEYSTORE` and `ANDROID_RELEASE_KEYSTORE_PASSWORD` of the GitHub environment `android-release`.
- That environment is deployable only from `main`, with the maintainer as its required reviewer, as `crates-io` is. No pull request's job can read its secrets. The maintainer's own agents may approve a deployment, as they act for the maintainer, so the protection rests on what can be built rather than on a second person: only a commit on `main`, under its branch ruleset, is built.
- Only a commit on `main` is built and signed, named by its SHA rather than by a tag, which can move: the commit whose `Release:` trailer and workspace version match the release's tag, and which the tag names. The tag must still name it when the APK is attached, and the `v*` tag ruleset blocks moving or deleting a release's tag.
- The build has no secret: it is unsigned. Signing runs on a fresh runner that never ran the build, and only its signing step has the keystore and its password, as files readable by their owner only, removed after the step whether or not it succeeds. The step runs only the SDK's `zipalign` and `apksigner`, which reads the password from its file, never from a command line; it compiles and runs none of the workspace's dependencies, whose build scripts would otherwise run beside the secrets.
- The release certificate's SHA-256 digest is committed in `android/release-certificate.sha256`, as `apksigner` prints it: 64 lowercase hexadecimal digits. Nothing is attached unless the APK is signed by that certificate alone, as `main` has it, is `com.codeotaku.tomori`, is aligned and is not debuggable, and its SHA-256 file matches it. The check fails if any tool it runs is missing or fails. Anyone can check a download against it.

Tomori's own Bake tasks do the Android steps, the same locally as in CI, configured by `[workspace.metadata.bake.android.apk]` in `Cargo.toml`, which names the package, the certificate file and the SDK build tools, and by that package's `[package.metadata.android]`, which names the application ID and the APK:

- `cargo bake android:apk:build OUTPUT` builds the release profile with `scripts/android.sh --release`, which `cargo-apk` signs with a throwaway key made for the build and removed after it, and writes it to OUTPUT with that signature removed.
- `cargo bake android:apk:sign KEYSTORE INPUT OUTPUT` aligns INPUT with `zipalign` and signs it with `apksigner`, with the password from the environment variable `ANDROID_RELEASE_KEYSTORE_PASSWORD`, or the one `--password-variable` names. It refuses a keystore that others than its owner can read.
- `cargo bake android:apk:verify APK [--certificate FILE]` checks an APK as above, against the committed certificate unless another is given.
- `cargo bake android:apk:release KEYSTORE OUTPUT` runs the three in turn, for a release made on one machine.

`.github/workflows/release-artifacts.yml` is bake-cargo's release artifacts workflow, which `cargo bake cargo:setup:artifacts --environment android-release --secrets ANDROID_RELEASE_KEYSTORE --secrets ANDROID_RELEASE_KEYSTORE_PASSWORD` generates, leaving `publish.yml` as bake-cargo's template. It runs after `publish.yml` succeeds for a push to `main`, or by hand from `main` for an existing release's tag, such as one made before the key was added; it finds and checks the release's commit with `cargo:release:artifacts:detect`, and attaches the APK, failing rather than replacing an asset that is already there. It runs Tomori's three scripts, each from a checkout of the release's commit:

- `scripts/release-artifacts/build` runs `cargo bake android:apk:build`, writing `tomori-VERSION.apk` unsigned.
- `scripts/release-artifacts/sign` decodes the keystore from its secret, aligns and signs the APK in place as `android:apk:sign` does, but with the SDK's tools alone, and writes `tomori-VERSION.apk.sha256`.
- `scripts/release-artifacts/verify` requires exactly those two files, checks the digest, and runs `cargo bake android:apk:verify` against `main`'s `android/release-certificate.sha256`.

On CI, `build` and `verify` install the tools they need, from `scripts/release-artifacts/android-tools.sh`, as the release's runners have only the SDK and stable Rust.

Every pull request runs the same three scripts, as the workflow runs them, with a throwaway key made and pinned in the job as the setup below makes the release key, and checks that the result passes, and that the unsigned build, a certificate other than the pinned one, another file and a debug build fail. It uploads nothing, and `test-result` requires it.

Local builds, `scripts/android.sh build` and `run`, stay in the dev profile, signed with the local debug key.

## Setup

The maintainer set up Tomori's release key this way in November 2026, before the first release to carry the APK; its certificate's digest is `8a055eaf0845c023bd7cd9afe9ff45275f10fdc6b8688759ca11e6767f1676bd`. A new key, should the key ever be replaced, is set up the same way:

1. Generates the key on a trusted machine, outside any checkout. Its distinguished name is published in every APK's certificate. `keytool` prompts for the password:

   ```sh
   keytool -genkeypair -v -keystore tomori-release.keystore -storetype PKCS12 \
     -alias tomori -keyalg RSA -keysize 4096 -validity 10000 -dname "CN=Tomori"
   ```

2. Commits its certificate's digest, in a pull request:

   ```sh
   keytool -list -v -keystore tomori-release.keystore -alias tomori \
     | sed -n 's/^[[:space:]]*SHA256: //p' | tr -d ':' | tr 'A-F' 'a-f' \
     > android/release-certificate.sha256
   ```

3. Creates the `android-release` environment, deployable only from `main`, with the maintainer as required reviewer, and adds the secrets `ANDROID_RELEASE_KEYSTORE`, the keystore base64-encoded, and `ANDROID_RELEASE_KEYSTORE_PASSWORD`, which `release-artifacts.yml` gives its sign step as the files `$RELEASE_SECRETS/ANDROID_RELEASE_KEYSTORE` and `$RELEASE_SECRETS/ANDROID_RELEASE_KEYSTORE_PASSWORD`.

4. Makes the `v*` tag ruleset block updates as well as deletion, as `cargo bake cargo:setup:github:apply` does.

## Rationale

People who use Tomori's headset need a build they can install without a Rust and Android toolchain, and #73 asked for the headset APK as a release artifact. Android identifies an app's updates by its signing certificate: losing the key means every installed copy must be uninstalled, losing its identity and tokens, before a newer one installs, and a leaked key lets anyone sign updates. So the key is held by the maintainer, behind a protected environment that only `main` can deploy; only an immutable commit on `main` is built; the build, whose dependencies' build scripts run code, never has the key; and the certificate is committed so that a wrong key is caught before anything is attached. The maintainer chose this on #132 in November 2026, with pull requests building the release profile with a throwaway key, and chose, on #166, Bake tasks for the Android steps, so that they are tested under the coverage gate and could become a Base crate, run by bake-cargo's release artifacts workflow rather than a workflow of Tomori's own.

## Scope

`.github/workflows/release-artifacts.yml`, `scripts/release-artifacts/`, the `android` and `android-release` jobs of `.github/workflows/test.yml`, the `android:apk:*` tasks in `bake/src/android.rs` and `bake/src/android/apk.rs`, `[workspace.metadata.bake.android.apk]`, `scripts/android.sh`'s `--release`, `android/release-certificate.sha256`, and the `android-release` environment and its secrets.

## Validation

- `tomori-bake`'s tests, under the coverage gate, run the tasks with fake build tools: a failing or missing `apksigner`, `zipalign` or `aapt2` fails the check, as do a debuggable APK, another package, another certificate and a second signer; signing passes the password through the environment only, and refuses a keystore others can read; the build removes its throwaway key and signature.
- A pull request's `android-release` job passes, running the three scripts, and its `android` job finds that the debug build fails the check for being debuggable.
- `release-artifacts.yml` is as `cargo bake cargo:setup:artifacts --environment android-release --secrets ANDROID_RELEASE_KEYSTORE --secrets ANDROID_RELEASE_KEYSTORE_PASSWORD` generates it. Inspect `scripts/release-artifacts/sign`: it runs nothing but `base64`, `zipalign`, `apksigner` and `sha256sum`.
- Inspect the `android-release` environment: the branch policy allows only `main`, and the maintainer is a required reviewer. Inspect the `v*` tag ruleset: it blocks updates and deletion.
- After a release, the GitHub Release has `tomori-VERSION.apk` and its `.sha256`, `sha256sum -c` passes, and `cargo bake android:apk:verify tomori-VERSION.apk` passes.
