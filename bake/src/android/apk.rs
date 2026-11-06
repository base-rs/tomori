// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Building, signing and checking a release APK, with the programs that `Tools` names. See `design/android-release.md`.
use bake::{Error, Result, Value};
use std::ffi::OsStr;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The alias of the throwaway key that `cargo-apk` signs a release build with, which names its JAR signature's files.
const BUILD_KEY_ALIAS: &str = "build";

/// The variable through which the keystore's password reaches `apksigner`, so that it is never on a command line.
const PASSWORD_VARIABLE: &str = "BAKE_ANDROID_KEYSTORE_PASSWORD";

/// The programs the operations run: the SDK's build tools, `keytool`, and the shell that runs `scripts/android.sh`.
pub struct Tools {
    pub build_tools: PathBuf,
    pub keytool: PathBuf,
    pub shell: PathBuf,
}

impl Tools {
    /// The build tools in `build_tools`, with `keytool` and `bash` from `PATH`.
    pub fn new(build_tools: PathBuf) -> Self {
        Self {
            build_tools,
            keytool: PathBuf::from("keytool"),
            shell: PathBuf::from("bash"),
        }
    }

    fn build_tool(&self, name: &str) -> Command {
        Command::new(self.build_tools.join(name))
    }
}

/// A headset application to release: its Cargo package, its application ID, where `cargo-apk` writes its APK in the release profile, and the file holding its release certificate's SHA-256 digest.
#[derive(Debug, PartialEq)]
pub struct Application {
    pub package: String,
    pub id: String,
    pub apk: PathBuf,
    pub certificate: PathBuf,
}

/// What `verify` found: the APK's application ID, and its one signer's certificate digest.
#[derive(Debug, PartialEq)]
pub struct Verification {
    pub application_id: String,
    pub certificate: String,
}

impl Verification {
    pub fn to_value(&self) -> Value {
        Value::from_iter([
            ("application_id", Value::from(self.application_id.as_str())),
            ("certificate", Value::from(self.certificate.as_str())),
        ])
    }
}

/// Run `command`, returning its standard output, or an error naming `what` with everything it printed, if it could not run or failed.
pub(crate) fn run(mut command: Command, what: &str) -> Result<String> {
    let output = command
        .output()
        .map_err(|error| Error::new(format!("could not run {what}: {error}")))?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }
    let printed = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let printed = printed.trim();
    let separator = if printed.is_empty() { "" } else { ":\n" };
    Err(Error::new(format!(
        "{what} failed, {}{separator}{printed}",
        output.status
    )))
}

/// Removes a file or directory when dropped, whether or not the operation that made it succeeded.
pub(crate) struct Removal(pub(crate) PathBuf);

impl Drop for Removal {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0).or_else(|_| fs::remove_file(&self.0));
    }
}

/// `path` with `suffix` appended to its file name.
pub(crate) fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// Build `application` in the release profile with `scripts/android.sh` in `root`, and write it to `output` unsigned and aligned.
///
/// `cargo-apk` signs every release build, so it signs this one with a throwaway key made for the build, in the target directory, and removed after it. Its signatures are then removed: the JAR signature's files, and the APK Signature Scheme block, which `zipalign` does not copy. The build is given no secret.
pub fn build(tools: &Tools, root: &Path, application: &Application, output: &Path) -> Result<()> {
    let profile = application
        .apk
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| {
            Error::new(format!(
                "{} is not in a profile's apk directory",
                application.apk.display()
            ))
        })?;
    let key = Removal(profile.join(format!("build-key-{}", std::process::id())));
    fs::create_dir_all(&key.0)?;
    let keystore = key.0.join("build.keystore");
    // The throwaway key's password is no secret: the key signs nothing that is kept.
    let password = ("CARGO_APK_RELEASE_KEYSTORE_PASSWORD", "throwaway");

    let mut keytool = Command::new(&tools.keytool);
    keytool
        .args([
            "-genkeypair",
            "-noprompt",
            "-storetype",
            "PKCS12",
            "-keyalg",
            "RSA",
            "-keysize",
            "2048",
            "-validity",
            "1",
        ])
        .args([
            "-alias",
            BUILD_KEY_ALIAS,
            "-dname",
            "CN=Throwaway build key",
        ])
        .arg("-keystore")
        .arg(&keystore)
        .args(["-storepass:env", password.0])
        .env(password.0, password.1);
    run(keytool, "keytool")?;

    match fs::remove_file(&application.apk) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error.into()),
        _ => {}
    }
    let mut android = Command::new(&tools.shell);
    android
        .current_dir(root)
        .args([
            "scripts/android.sh",
            "--package",
            &application.package,
            "--release",
            "build",
        ])
        .env("CARGO_APK_RELEASE_KEYSTORE", &keystore)
        .env(password.0, password.1)
        // The build's progress, all on standard error, as it happens.
        .stdout(Stdio::from(std::io::stderr()))
        .stderr(Stdio::inherit());
    run(android, "scripts/android.sh")?;

    let stripped = Removal(with_suffix(output, ".unaligned"));
    fs::create_dir_all(output.parent().unwrap_or(Path::new(".")))?;
    fs::copy(&application.apk, &stripped.0)?;
    let mut remove = tools.build_tool("aapt");
    remove.arg("remove").arg(&stripped.0).args([
        format!("META-INF/{}.SF", BUILD_KEY_ALIAS.to_uppercase()),
        format!("META-INF/{}.RSA", BUILD_KEY_ALIAS.to_uppercase()),
        "META-INF/MANIFEST.MF".to_owned(),
    ]);
    run(remove, "aapt remove")?;
    align(tools, &stripped.0, output)?;

    let mut verify = tools.build_tool("apksigner");
    verify.arg("verify").arg(output);
    if run(verify, "apksigner verify").is_ok() {
        return Err(Error::new(format!("{} is still signed", output.display())));
    }
    Ok(())
}

/// Align `input` into `output` with `zipalign`, overwriting it.
fn align(tools: &Tools, input: &Path, output: &Path) -> Result<()> {
    let mut zipalign = tools.build_tool("zipalign");
    zipalign.args(["-f", "4"]).arg(input).arg(output);
    run(zipalign, "zipalign").map(drop)
}

/// Align `input` and sign it into `output` with the one key in `keystore`, a PKCS12 keystore whose key's password is `password`, replacing any signature it had.
///
/// The keystore must be readable by its owner only. The password reaches `apksigner` through its environment, not its command line.
pub fn sign(
    tools: &Tools,
    keystore: &Path,
    password: &OsStr,
    input: &Path,
    output: &Path,
) -> Result<()> {
    let mode = fs::metadata(keystore)
        .map_err(|error| Error::new(format!("{}: {error}", keystore.display())))?
        .permissions()
        .mode();
    if mode & 0o077 != 0 {
        return Err(Error::new(format!(
            "{} is readable by others than its owner; `chmod 600` it",
            keystore.display()
        )));
    }
    if password.is_empty() {
        return Err(Error::new("the keystore's password is empty"));
    }
    fs::create_dir_all(output.parent().unwrap_or(Path::new(".")))?;
    let aligned = Removal(with_suffix(output, ".aligned"));
    align(tools, input, &aligned.0)?;
    let mut apksigner = tools.build_tool("apksigner");
    apksigner
        .args(["sign", "--v4-signing-enabled", "false", "--ks"])
        .arg(keystore)
        .args(["--ks-pass", &format!("env:{PASSWORD_VARIABLE}")])
        .env(PASSWORD_VARIABLE, password)
        .arg("--in")
        .arg(&aligned.0)
        .arg("--out")
        .arg(output);
    run(apksigner, "apksigner sign").map(drop)
}

/// A SHA-256 digest as `apksigner` prints it, 64 lowercase hexadecimal digits, from `text`, which may use upper case and colons, as `keytool` prints it.
pub fn parse_digest(text: &str) -> Result<String> {
    let digest: String = text
        .chars()
        .filter(|character| !character.is_whitespace() && *character != ':')
        .map(|character| character.to_ascii_lowercase())
        .collect();
    if digest.len() == 64
        && digest
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        Ok(digest)
    } else {
        Err(Error::new(format!("{text:?} is not a SHA-256 digest")))
    }
}

/// The certificate digests of each signer that `apksigner verify --print-certs` printed.
pub fn signer_digests(certificates: &str) -> Vec<&str> {
    certificates
        .lines()
        .filter_map(|line| line.strip_prefix("Signer #"))
        .filter_map(|line| line.split_once(" certificate SHA-256 digest: "))
        .map(|(_, digest)| digest.trim())
        .collect()
}

/// The application ID, and whether it is debuggable, from `aapt2 dump badging`, which must name one.
pub fn parse_badging(badging: &str) -> Result<(String, bool)> {
    let id = badging
        .lines()
        .find_map(|line| line.strip_prefix("package: name='"))
        .and_then(|line| line.split_once('\''))
        .map(|(id, _)| id.to_owned())
        .ok_or_else(|| Error::new("aapt2 named no package"))?;
    let debuggable = badging
        .lines()
        .any(|line| line.starts_with("application-debuggable"));
    Ok((id, debuggable))
}

/// Check that `apk` is `application_id`, not debuggable, aligned, and signed by one signer, whose certificate has the digest in `certificate`. Fails if any tool is missing or fails.
pub fn verify(
    tools: &Tools,
    apk: &Path,
    certificate: &Path,
    application_id: &str,
) -> Result<Verification> {
    let expected = fs::read_to_string(certificate)
        .map_err(|error| Error::new(format!("{}: {error}", certificate.display())))?;
    let expected = parse_digest(&expected)
        .map_err(|error| Error::new(format!("{}: {error}", certificate.display())))?;
    let failure = |message: String| Error::new(format!("{}: {message}", apk.display()));

    let mut apksigner = tools.build_tool("apksigner");
    apksigner.args(["verify", "--print-certs"]).arg(apk);
    let certificates = run(apksigner, "apksigner verify")?;
    let digests = signer_digests(&certificates);
    if digests != [expected.as_str()] {
        return Err(failure(format!(
            "is signed by [{}], not by {expected} alone, the certificate in {}",
            digests.join(", "),
            certificate.display()
        )));
    }

    let mut zipalign = tools.build_tool("zipalign");
    zipalign.args(["-c", "4"]).arg(apk);
    run(zipalign, "zipalign -c")?;

    // The whole badging, read before it is matched, so that a failure of aapt2's fails the check.
    let mut aapt2 = tools.build_tool("aapt2");
    aapt2.args(["dump", "badging"]).arg(apk);
    let (id, debuggable) = parse_badging(&run(aapt2, "aapt2 dump badging")?)?;
    if id != application_id {
        return Err(failure(format!("is {id}, not {application_id}")));
    }
    if debuggable {
        return Err(failure(
            "is debuggable, which a release must not be".to_owned(),
        ));
    }
    Ok(Verification {
        application_id: id,
        certificate: expected,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::Permissions;
    use tempfile::TempDir;

    const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    const OTHER: &str = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";

    /// A directory with fake build tools, `keytool` and `scripts/android.sh`, each recording its arguments in `NAME.args`, and which by default behave as for a good release.
    struct Fixture {
        directory: TempDir,
    }

    impl Fixture {
        fn new() -> Self {
            let fixture = Self {
                directory: tempfile::tempdir().unwrap(),
            };
            fixture.tool(
                "apksigner",
                &format!(
                    r#"case "$1" in
    verify) [[ "$2" == --print-certs ]] || {{ echo "DOES NOT VERIFY"; exit 1; }}
        echo "Signer #1 certificate DN: CN=Tomori"; echo "Signer #1 certificate SHA-256 digest: {DIGEST}" ;;
    sign) printf '%s' "$BAKE_ANDROID_KEYSTORE_PASSWORD" > "$0.password"
        while (($#)); do case "$1" in --in) input="$2" ;; --out) output="$2" ;; esac; shift; done
        cp "$input" "$output" ;;
esac"#
                ),
            );
            fixture.tool("zipalign", r#"if [[ "$1" == -f ]]; then cp "$3" "$4"; fi"#);
            fixture.tool("aapt", "");
            fixture.tool("aapt2", "echo \"package: name='com.codeotaku.tomori' versionCode='1'\"; echo \"application-label:'Tomori'\"");
            fixture.tool("keytool", r#"while (($#)); do [[ "$1" == -keystore ]] && echo "$CARGO_APK_RELEASE_KEYSTORE_PASSWORD" > "$2"; shift; done"#);
            fixture.write(
                "scripts/android.sh",
                &format!(
                    "printf '%s\\n' \"$*\" > android.args\n[[ -f \"$CARGO_APK_RELEASE_KEYSTORE\" ]] || exit 3\necho \"$CARGO_APK_RELEASE_KEYSTORE\" > android.keystore\nmkdir -p target/release/apk && echo apk > {}\n",
                    fixture.application().apk.display()
                ),
            );
            fixture
        }

        fn path(&self, name: &str) -> PathBuf {
            self.directory.path().join(name)
        }

        fn write(&self, name: &str, contents: &str) -> PathBuf {
            let path = self.path(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, contents).unwrap();
            path
        }

        /// A fake tool, running `body` after recording its arguments.
        ///
        /// It is written by `install`, not by this process: a process that other tests' threads fork while it holds a file open for writing passes the descriptor to the child until it runs its program, and running the file meanwhile fails with "Text file busy". `install`'s descriptor is closed when it exits, before the tool runs.
        fn tool(&self, name: &str, body: &str) {
            let source = self.write(
                &format!("sources/{name}"),
                &format!("#!/usr/bin/env bash\nprintf '%s\\n' \"$*\" >> \"$0.args\"\n{body}\n"),
            );
            let path = self.path(&format!("tools/{name}"));
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let installed = Command::new("install")
                .args(["-m", "755"])
                .arg(source)
                .arg(path)
                .status()
                .unwrap();
            assert!(installed.success());
        }

        fn arguments(&self, name: &str) -> String {
            fs::read_to_string(self.path(&format!("tools/{name}.args"))).unwrap_or_default()
        }

        fn tools(&self) -> Tools {
            Tools {
                keytool: self.path("tools/keytool"),
                ..Tools::new(self.path("tools"))
            }
        }

        fn application(&self) -> Application {
            Application {
                package: "tomori-client-quest".to_owned(),
                id: "com.codeotaku.tomori".to_owned(),
                apk: self.path("target/release/apk/tomori.apk"),
                certificate: self.path("release-certificate.sha256"),
            }
        }

        fn verify(&self) -> Result<Verification> {
            let apk = self.write("tomori.apk", "apk");
            let certificate = self.write("release-certificate.sha256", &format!("{DIGEST}\n"));
            verify(&self.tools(), &apk, &certificate, "com.codeotaku.tomori")
        }

        fn keystore(&self, mode: u32) -> PathBuf {
            let keystore = self.write("release.keystore", "key");
            fs::set_permissions(&keystore, Permissions::from_mode(mode)).unwrap();
            keystore
        }

        fn sign(&self, password: &str) -> Result<()> {
            let input = self.write("unsigned.apk", "apk");
            sign(
                &self.tools(),
                &self.keystore(0o600),
                OsStr::new(password),
                &input,
                &self.path("out/signed.apk"),
            )
        }

        fn build(&self) -> Result<()> {
            build(
                &self.tools(),
                self.directory.path(),
                &self.application(),
                &self.path("out/unsigned.apk"),
            )
        }
    }

    fn message(result: Result<impl std::fmt::Debug>) -> String {
        result.unwrap_err().to_string()
    }

    #[test]
    fn digests_are_read_as_apksigner_prints_them() {
        assert_eq!(
            parse_digest(&format!(" {}\n", DIGEST.to_uppercase())).unwrap(),
            DIGEST
        );
        let colons: Vec<_> = DIGEST
            .as_bytes()
            .chunks(2)
            .map(|pair| std::str::from_utf8(pair).unwrap())
            .collect();
        assert_eq!(parse_digest(&colons.join(":")).unwrap(), DIGEST);
        assert!(message(parse_digest("")).contains("is not a SHA-256 digest"));
        assert!(parse_digest(&DIGEST[1..]).is_err());
        assert!(parse_digest(&DIGEST.replace('a', "g")).is_err());
    }

    #[test]
    fn signers_and_badging_are_read_from_the_tools_output() {
        let certificates = format!(
            "Signer #1 certificate DN: CN=A\nSigner #1 certificate SHA-256 digest: {DIGEST}\nSigner #2 certificate SHA-256 digest: {OTHER}\nWARNING: Signer #3 certificate SHA-256 digest: {OTHER}\n"
        );
        assert_eq!(signer_digests(&certificates), [DIGEST, OTHER]);
        assert_eq!(
            parse_badging("package: name='com.example' versionCode='1'\napplication-debuggable\n")
                .unwrap(),
            ("com.example".to_owned(), true)
        );
        assert_eq!(
            parse_badging("package: name='com.example'\n").unwrap(),
            ("com.example".to_owned(), false)
        );
        assert_eq!(message(parse_badging("")), "aapt2 named no package");
        assert!(parse_badging("package: name='unterminated").is_err());
    }

    #[test]
    fn a_failing_program_reports_what_it_printed() {
        let fixture = Fixture::new();
        fixture.tool("printing", "echo out; echo err >&2; exit 4");
        fixture.tool("silent", "exit 5");
        let tools = fixture.tools();
        assert_eq!(
            message(run(tools.build_tool("printing"), "printing")),
            "printing failed, exit status: 4:\nout\nerr"
        );
        assert_eq!(
            message(run(tools.build_tool("silent"), "silent")),
            "silent failed, exit status: 5"
        );
        assert!(
            message(run(tools.build_tool("missing"), "missing"))
                .starts_with("could not run missing: ")
        );
        assert_eq!(
            with_suffix(Path::new("a/b.apk"), ".x"),
            Path::new("a/b.apk.x")
        );
    }

    #[test]
    fn a_release_apk_passes() {
        let fixture = Fixture::new();
        let verification = fixture.verify().unwrap();
        assert_eq!(
            verification.to_value(),
            Value::from_iter([
                ("application_id", "com.codeotaku.tomori"),
                ("certificate", DIGEST)
            ])
        );
        assert_eq!(
            fixture.arguments("zipalign").trim(),
            format!("-c 4 {}", fixture.path("tomori.apk").display())
        );
    }

    #[test]
    fn verification_fails_when_a_tool_is_missing_or_fails() {
        for tool in ["apksigner", "zipalign", "aapt2"] {
            let fixture = Fixture::new();
            fs::remove_file(fixture.path(&format!("tools/{tool}"))).unwrap();
            assert!(
                message(fixture.verify()).starts_with("could not run"),
                "{tool}"
            );

            let fixture = Fixture::new();
            // A failing aapt2 that printed the badging of a release must still fail the check.
            fixture.tool(
                tool,
                "echo \"package: name='com.codeotaku.tomori'\"; exit 1",
            );
            assert!(
                message(fixture.verify()).contains("failed, exit status: 1"),
                "{tool}"
            );
        }
    }

    #[test]
    fn verification_fails_for_an_apk_that_is_not_a_release() {
        let fixture = Fixture::new();
        fixture.tool(
            "apksigner",
            &format!("echo 'Signer #1 certificate SHA-256 digest: {OTHER}'"),
        );
        assert!(
            message(fixture.verify())
                .contains(&format!("is signed by [{OTHER}], not by {DIGEST} alone"))
        );

        fixture.tool("apksigner", &format!("echo 'Signer #1 certificate SHA-256 digest: {DIGEST}'; echo 'Signer #2 certificate SHA-256 digest: {OTHER}'"));
        assert!(message(fixture.verify()).contains(&format!("is signed by [{DIGEST}, {OTHER}]")));

        fixture.tool("apksigner", "");
        assert!(message(fixture.verify()).contains("is signed by []"));

        let fixture = Fixture::new();
        fixture.tool("aapt2", "");
        assert_eq!(message(fixture.verify()), "aapt2 named no package");

        fixture.tool("aapt2", "echo \"package: name='com.example.other'\"");
        assert!(
            message(fixture.verify()).ends_with("is com.example.other, not com.codeotaku.tomori")
        );

        fixture.tool(
            "aapt2",
            "echo \"package: name='com.codeotaku.tomori'\"; echo application-debuggable",
        );
        assert!(message(fixture.verify()).ends_with("is debuggable, which a release must not be"));
    }

    #[test]
    fn verification_needs_the_release_certificate() {
        let fixture = Fixture::new();
        let apk = fixture.write("tomori.apk", "apk");
        let missing = verify(
            &fixture.tools(),
            &apk,
            &fixture.path("missing.sha256"),
            "com.codeotaku.tomori",
        );
        assert!(
            message(missing)
                .starts_with(&format!("{}: ", fixture.path("missing.sha256").display()))
        );
        let malformed = fixture.write("malformed.sha256", "not a digest");
        assert!(
            message(verify(
                &fixture.tools(),
                &apk,
                &malformed,
                "com.codeotaku.tomori"
            ))
            .ends_with("is not a SHA-256 digest")
        );
        assert_eq!(fixture.arguments("apksigner"), "");
    }

    #[test]
    fn signing_aligns_and_signs_with_the_password_out_of_the_command_line() {
        let fixture = Fixture::new();
        fixture.sign("correct horse").unwrap();
        assert_eq!(
            fs::read_to_string(fixture.path("out/signed.apk")).unwrap(),
            "apk"
        );
        assert!(!fixture.path("out/signed.apk.aligned").exists());
        assert_eq!(
            fs::read_to_string(fixture.path("tools/apksigner.password")).unwrap(),
            "correct horse"
        );
        let arguments = fixture.arguments("apksigner");
        assert!(!arguments.contains("correct horse"));
        assert!(arguments.starts_with(&format!(
            "sign --v4-signing-enabled false --ks {} --ks-pass env:BAKE_ANDROID_KEYSTORE_PASSWORD --in ",
            fixture.path("release.keystore").display()
        )));
        assert!(fixture.arguments("zipalign").starts_with("-f 4 "));
    }

    #[test]
    fn signing_fails_closed() {
        let fixture = Fixture::new();
        assert_eq!(
            message(fixture.sign("")),
            "the keystore's password is empty"
        );

        let input = fixture.write("unsigned.apk", "apk");
        let output = fixture.path("signed.apk");
        let shared = sign(
            &fixture.tools(),
            &fixture.keystore(0o640),
            OsStr::new("password"),
            &input,
            &output,
        );
        assert!(message(shared).ends_with("is readable by others than its owner; `chmod 600` it"));
        let missing = sign(
            &fixture.tools(),
            &fixture.path("missing.keystore"),
            OsStr::new("password"),
            &input,
            &output,
        );
        assert!(
            message(missing)
                .starts_with(&format!("{}: ", fixture.path("missing.keystore").display()))
        );
        assert_eq!(fixture.arguments("zipalign"), "");

        fixture.tool("zipalign", "exit 1");
        assert!(message(fixture.sign("password")).starts_with("zipalign failed"));
        assert_eq!(fixture.arguments("apksigner"), "");

        let fixture = Fixture::new();
        fixture.tool("apksigner", "exit 1");
        assert!(message(fixture.sign("password")).starts_with("apksigner sign failed"));
        assert!(!fixture.path("out/signed.apk.aligned").exists());
    }

    #[test]
    fn a_release_build_is_unsigned_and_made_without_a_secret() {
        let fixture = Fixture::new();
        fixture.build().unwrap();
        assert_eq!(
            fs::read_to_string(fixture.path("out/unsigned.apk")).unwrap(),
            "apk\n"
        );
        assert_eq!(
            fs::read_to_string(fixture.path("android.args")).unwrap(),
            "--package tomori-client-quest --release build\n"
        );
        // The throwaway key was given to the build, and removed after it, with the build's other files.
        let keystore = fs::read_to_string(fixture.path("android.keystore")).unwrap();
        assert!(
            keystore
                .trim()
                .starts_with(fixture.path("target/release/build-key-").to_str().unwrap())
        );
        assert!(!Path::new(keystore.trim()).parent().unwrap().exists());
        assert!(!fixture.path("out/unsigned.apk.unaligned").exists());
        assert!(fixture.arguments("keytool").contains("-alias build "));
        assert!(fixture.arguments("aapt").starts_with("remove "));
        assert!(
            fixture
                .arguments("aapt")
                .trim()
                .ends_with("META-INF/BUILD.SF META-INF/BUILD.RSA META-INF/MANIFEST.MF")
        );
        assert_eq!(
            fixture.arguments("apksigner").trim(),
            format!("verify {}", fixture.path("out/unsigned.apk").display())
        );
    }

    #[test]
    fn a_release_build_fails_when_a_step_fails() {
        let fixture = Fixture::new();
        fixture.tool("keytool", "exit 1");
        assert!(message(fixture.build()).starts_with("keytool failed"));
        assert!(!fixture.path("android.args").exists());

        let fixture = Fixture::new();
        fixture.write("scripts/android.sh", "exit 2");
        assert_eq!(
            message(fixture.build()),
            "scripts/android.sh failed, exit status: 2"
        );

        let fixture = Fixture::new();
        fixture.write("scripts/android.sh", "true");
        assert!(fixture.build().is_err(), "no APK was built");

        let fixture = Fixture::new();
        fs::create_dir_all(fixture.path("target/release/apk/tomori.apk")).unwrap();
        assert!(
            fixture.build().is_err(),
            "the previous APK could not be removed"
        );

        let fixture = Fixture::new();
        fixture.tool("aapt", "exit 1");
        assert!(message(fixture.build()).starts_with("aapt remove failed"));

        let fixture = Fixture::new();
        fixture.tool("zipalign", "exit 1");
        assert!(message(fixture.build()).starts_with("zipalign failed"));
        assert!(!fixture.path("out/unsigned.apk.unaligned").exists());

        let fixture = Fixture::new();
        fixture.tool("apksigner", r#"if [[ "$1" == verify ]]; then exit 0; fi"#);
        assert_eq!(
            message(fixture.build()),
            format!(
                "{} is still signed",
                fixture.path("out/unsigned.apk").display()
            )
        );

        let fixture = Fixture::new();
        let application = Application {
            apk: PathBuf::from("tomori.apk"),
            ..fixture.application()
        };
        let unplaced = build(
            &fixture.tools(),
            fixture.directory.path(),
            &application,
            &fixture.path("out.apk"),
        );
        assert_eq!(
            message(unplaced),
            "tomori.apk is not in a profile's apk directory"
        );
    }

    #[test]
    fn nothing_is_built_or_signed_where_its_directory_cannot_be_made() {
        let fixture = Fixture::new();
        let file = fixture.write("file", "");
        let application = Application {
            apk: file.join("release/apk/tomori.apk"),
            ..fixture.application()
        };
        assert!(
            build(
                &fixture.tools(),
                fixture.directory.path(),
                &application,
                &fixture.path("out.apk")
            )
            .is_err()
        );
        assert_eq!(fixture.arguments("keytool"), "");

        let output = file.join("unsigned.apk");
        assert!(
            build(
                &fixture.tools(),
                fixture.directory.path(),
                &fixture.application(),
                &output
            )
            .is_err()
        );
        assert_eq!(fixture.arguments("aapt"), "");

        let input = fixture.write("unsigned.apk", "apk");
        assert!(
            sign(
                &fixture.tools(),
                &fixture.keystore(0o600),
                OsStr::new("password"),
                &input,
                &output
            )
            .is_err()
        );
        assert_eq!(fixture.arguments("zipalign"), "");
    }
}
