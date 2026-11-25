//! `cargo xtask package [--out DIR]`: the download for the system it runs on.
//!
//! One archive with the app, the command line, the shipped library and every
//! notice: a tar.xz with a menu entry and a file type on Linux, a zip on
//! Windows, a zip with Sparksmith.app on macOS. The programs are built with
//! the `dist` profile. Nothing is published, and nothing is signed but the
//! ad-hoc signature macOS asks of every arm64 program. The licence gates run
//! first. `docs/packages.md` has the rest.
mod notices;

use crate::{Result, brand_crates, cargo_metadata_target, licences, root};
use sparksmith_core::product;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The programs: their packages and their binaries.
const PROGRAMS: [(&str, &str); 2] = [
    ("sparksmith", "sparksmith-desktop"),
    ("sparksmith-cli", product::SHORT_NAME),
];
const PROFILE: &str = "dist";
const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Licence files of the repository that every package holds.
const OWN_NOTICES: [&str; 4] = ["LICENSE-MIT", "LICENSE-APACHE", "NOTICE", "ATTRIBUTION.md"];
/// The file of the crates' licences and notices.
const THIRD_PARTY: &str = "THIRD-PARTY-NOTICES.txt";

pub(crate) fn run(args: &[String]) -> Result<()> {
    let out = match args {
        [] => None,
        [flag, dir] if flag == "--out" => Some(PathBuf::from(dir)),
        _ => return Err("usage: cargo xtask package [--out DIR]".into()),
    };
    licences::check()?;
    brand_crates::check()?;
    let root = root()?;
    let target = host()?;
    let commit = commit(&root)?;
    let built = build(&root, &target, &commit)?;
    let out = match out {
        Some(dir) => dir,
        None => cargo_metadata_target()?.join("package"),
    };
    fs::create_dir_all(&out)?;
    let work = tempfile::tempdir_in(&out)?;
    let package = Package {
        root: &root,
        built: &built,
        notices: notices::text(&target)?,
        version: format!("{VERSION} ({commit})"),
        commit: &commit,
    };
    let name = format!("{}-{VERSION}-{commit}-{}", product::SHORT_NAME, platform()?);
    let archive = package.make(&work.path().join(&name), &out, &name)?;
    println!(
        "{}: {} bytes",
        archive.display(),
        fs::metadata(&archive)?.len()
    );
    for (_, binary) in PROGRAMS {
        let file = built.join(exe(binary));
        println!("{binary}: {} bytes", fs::metadata(&file)?.len());
    }
    Ok(())
}

/// The system a package is for, as its file name says it.
fn platform() -> Result<&'static str> {
    Ok(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "linux-x86_64",
        ("windows", "x86_64") => "windows-x86_64",
        ("macos", "aarch64") => "macos-arm64",
        (os, arch) => return Err(format!("no package for {os} on {arch}").into()),
    })
}

fn exe(name: &str) -> String {
    format!("{name}{}", std::env::consts::EXE_SUFFIX)
}

fn output(command: &mut Command) -> Result<String> {
    let output = command.output()?;
    if !output.status.success() {
        return Err(format!("{command:?}: {}", String::from_utf8_lossy(&output.stderr)).into());
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

/// The target triple of this machine's compiler.
fn host() -> Result<String> {
    let version = output(Command::new("rustc").arg("-vV"))?;
    version
        .lines()
        .find_map(|l| l.strip_prefix("host: "))
        .map(str::to_owned)
        .ok_or_else(|| "rustc names no host".into())
}

/// The commit packaged, with `-dirty` when the tree differs from it.
fn commit(root: &Path) -> Result<String> {
    let git = |args: &[&str]| output(Command::new("git").args(args).current_dir(root));
    let commit = git(&["rev-parse", "--short=10", "HEAD"])?;
    let changed = !git(&["status", "--porcelain"])?.is_empty();
    Ok(if changed {
        format!("{commit}-dirty")
    } else {
        commit
    })
}

/// Build the programs for `target`; the folder they are in.
fn build(root: &Path, target: &str, commit: &str) -> Result<PathBuf> {
    // No path of the machine that built it: panic messages would name them.
    let mut flags = vec![remap(root, ".")];
    for (variable, folder) in [("CARGO_HOME", ".cargo"), ("RUSTUP_HOME", ".rustup")] {
        let home = std::env::var_os(variable).map(PathBuf::from).or_else(|| {
            let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
            Some(Path::new(&home).join(folder))
        });
        if let Some(home) = home {
            flags.push(remap(&home, folder));
        }
    }
    if cfg!(windows) {
        // No Visual C++ runtime to install first.
        flags.push("-Ctarget-feature=+crt-static".to_owned());
    }
    if cfg!(target_os = "linux") {
        // The loader of every distribution; a Nix compiler names its own.
        flags.push("-Clink-arg=-Wl,--dynamic-linker=/lib64/ld-linux-x86-64.so.2".to_owned());
    }
    let mut command = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    command
        .args([
            "build",
            "--locked",
            "--profile",
            PROFILE,
            "--target",
            target,
        ])
        .env("CARGO_ENCODED_RUSTFLAGS", flags.join("\u{1f}"))
        .env("NIX_DONT_SET_RPATH", "1")
        .env("SPARKSMITH_COMMIT", commit)
        .env("NIX_LDFLAGS", without_rpath())
        .env("MACOSX_DEPLOYMENT_TARGET", "13.0")
        .current_dir(root);
    for (package, _) in PROGRAMS {
        command.args(["-p", package]);
    }
    if !command.status()?.success() {
        return Err("the programs do not build".into());
    }
    let built = cargo_metadata_target()?.join(target).join(PROFILE);
    for (_, binary) in PROGRAMS {
        let program = fs::read(built.join(exe(binary)))?;
        let place = root.to_string_lossy();
        if program.windows(place.len()).any(|w| w == place.as_bytes()) {
            return Err(format!("{binary} names the folder it was built in, {place}").into());
        }
    }
    Ok(built)
}

/// What a Nix shell hands the linker, without the library paths it would
/// write into the programs; empty elsewhere.
fn without_rpath() -> String {
    let flags = std::env::var("NIX_LDFLAGS").unwrap_or_default();
    let mut kept = Vec::new();
    let mut words = flags.split_whitespace();
    while let Some(word) = words.next() {
        if word == "-rpath" {
            words.next();
        } else {
            kept.push(word);
        }
    }
    kept.join(" ")
}

fn remap(from: &Path, to: &str) -> String {
    format!("--remap-path-prefix={}={to}", from.display())
}

struct Package<'a> {
    root: &'a Path,
    built: &'a Path,
    notices: String,
    version: String,
    commit: &'a str,
}

impl Package<'_> {
    #[cfg(target_os = "linux")]
    fn make(&self, dir: &Path, out: &Path, name: &str) -> Result<PathBuf> {
        let from = self.root.join("packaging/linux");
        let id = product::BUNDLE_ID;
        self.programs(&dir.join("bin"))?;
        let share = dir.join("share");
        self.library(&share.join(product::SHORT_NAME).join("library"))?;
        self.licences(&share.join("doc").join(product::SHORT_NAME))?;
        let place = |source: PathBuf, target: PathBuf| copy(&source, &target);
        place(
            from.join(format!("{id}.desktop")),
            share.join(format!("applications/{id}.desktop")),
        )?;
        place(
            from.join(format!("{id}.xml")),
            share.join(format!("mime/packages/{id}.xml")),
        )?;
        place(
            self.root.join("apps/sparksmith/macos/AppIcon.svg"),
            share.join(format!("icons/hicolor/scalable/apps/{id}.svg")),
        )?;
        place(from.join("install.sh"), dir.join("install.sh"))?;
        let glibc = PROGRAMS
            .iter()
            .filter_map(|(_, binary)| glibc(&fs::read(dir.join("bin").join(binary)).ok()?))
            .max()
            .ok_or("the programs name no glibc version")?;
        fs::write(
            dir.join("README.txt"),
            self.readme(
                "Linux x86_64",
                "Unpack it anywhere and start bin/sparksmith-desktop [FILE.sparksmith].\n\
                 For a menu entry and the .sparksmith file type: sh install.sh [PREFIX]\n\
                 (PREFIX is ~/.local unless given). bin/sparksmith is the command line.",
                &format!(
                    "glibc {}.{} or newer; a Wayland or X11 desktop with libxkbcommon; a Vulkan\n\
                     driver (Mesa's, NVIDIA's) or OpenGL through EGL; for file dialogs\n\
                     xdg-desktop-portal with a backend (GTK, KDE or GNOME).",
                    glibc.0, glibc.1
                ),
                [
                    "$XDG_CONFIG_HOME/sparksmith, else ~/.config/sparksmith",
                    "$XDG_STATE_HOME/sparksmith/recovery, else ~/.local/state/sparksmith/recovery",
                    "$XDG_CACHE_HOME/sparksmith, else ~/.cache/sparksmith",
                ],
                "Delete the unpacked folder; if installed: sh install.sh --uninstall [PREFIX]",
                "The texts are in share/doc/sparksmith.",
            )?,
        )?;
        let archive = out.join(format!("{name}.tar.xz"));
        let parent = dir.parent().ok_or("no folder to pack")?;
        // The same names and modes whoever packs it.
        output(
            Command::new("tar")
                .env("XZ_OPT", "-6 -T0")
                .args([
                    "--sort=name",
                    "--owner=0",
                    "--group=0",
                    "--numeric-owner",
                    "-cJf",
                ])
                .arg(&archive)
                .arg("-C")
                .arg(parent)
                .arg(name),
        )?;
        Ok(archive)
    }

    #[cfg(windows)]
    fn make(&self, dir: &Path, out: &Path, name: &str) -> Result<PathBuf> {
        self.programs(dir)?;
        self.library(&dir.join("library"))?;
        self.licences(&dir.join("licences"))?;
        copy(
            &self.root.join("packaging/windows/file-type.ps1"),
            &dir.join("file-type.ps1"),
        )?;
        let readme = self.readme(
            "Windows x86_64",
            "Unpack it anywhere and start sparksmith-desktop.exe [FILE.sparksmith].\n\
             For the .sparksmith file type (this user only):\n\
             powershell -ExecutionPolicy Bypass -File file-type.ps1\n\
             sparksmith.exe is the command line. The programs are not signed: Windows\n\
             SmartScreen may ask before the first start.",
            "Windows 10 or 11 with a Direct3D 12 or Vulkan driver.",
            [
                "%APPDATA%\\sparksmith",
                "%LOCALAPPDATA%\\Sparksmith\\Recovery",
                "%LOCALAPPDATA%\\sparksmith\\cache",
            ],
            "file-type.ps1 -Remove, if it was run; then delete the unpacked folder.",
            "The texts are in licences.",
        )?;
        fs::write(dir.join("README.txt"), readme.replace('\n', "\r\n"))?;
        zip(dir, &out.join(format!("{name}.zip")))
    }

    #[cfg(target_os = "macos")]
    fn make(&self, dir: &Path, out: &Path, name: &str) -> Result<PathBuf> {
        let app = dir.join(format!("{}.app", product::NAME));
        let contents = app.join("Contents");
        self.programs(&contents.join("MacOS"))?;
        let resources = contents.join("Resources");
        self.library(&resources.join("library"))?;
        self.licences(&resources.join("licences"))?;
        let macos = self.root.join("apps/sparksmith/macos");
        copy(&macos.join("AppIcon.icns"), &resources.join("AppIcon.icns"))?;
        let plist = fs::read_to_string(macos.join("Info.plist"))?;
        // Three numbers at most are allowed there: the commit is in About.
        let count = output(
            Command::new("git")
                .args(["rev-list", "--count", "HEAD"])
                .current_dir(self.root),
        )?;
        fs::write(
            contents.join("Info.plist"),
            versioned(&plist, VERSION, &count)?,
        )?;
        fs::write(contents.join("PkgInfo"), "APPL????")?;
        fs::write(
            dir.join("README.txt"),
            self.readme(
                "macOS arm64",
                "Move Sparksmith.app to Applications and open it. It is not notarised by Apple:\n\
                 the first time, macOS refuses it; allow it in System Settings > Privacy &\n\
                 Security > Open Anyway. The command line is\n\
                 Sparksmith.app/Contents/MacOS/sparksmith.",
                "macOS 13 or newer on Apple silicon.",
                [
                    "~/Library/Application Support/sparksmith",
                    "~/Library/Application Support/Sparksmith/Recovery",
                    "~/Library/Caches/sparksmith",
                ],
                "Move Sparksmith.app to the Bin.",
                "The texts are in Sparksmith.app/Contents/Resources/licences.",
            )?,
        )?;
        // Ad hoc: no identity, only what arm64 programs need to start.
        for program in [contents.join("MacOS").join(product::SHORT_NAME), app] {
            output(
                Command::new("codesign")
                    .args(["--force", "--sign", "-"])
                    .arg(&program),
            )?;
        }
        let archive = out.join(format!("{name}.zip"));
        output(
            Command::new("ditto")
                .args(["-c", "-k", "--norsrc", "--keepParent"])
                .arg(dir)
                .arg(&archive),
        )?;
        Ok(archive)
    }

    fn programs(&self, dir: &Path) -> Result<()> {
        for (_, binary) in PROGRAMS {
            copy(&self.built.join(exe(binary)), &dir.join(exe(binary)))?;
        }
        Ok(())
    }

    /// The shipped library as it is committed, without what else lies there.
    fn library(&self, dir: &Path) -> Result<()> {
        let files = output(
            Command::new("git")
                .args(["-c", "core.quotepath=off", "ls-files", "library"])
                .current_dir(self.root),
        )?;
        if files.is_empty() {
            return Err("git lists no library files".into());
        }
        for file in files.lines() {
            let inside = Path::new(file).strip_prefix("library")?;
            copy(&self.root.join(file), &dir.join(inside))?;
        }
        Ok(())
    }

    fn licences(&self, dir: &Path) -> Result<()> {
        for file in OWN_NOTICES {
            copy(&self.root.join(file), &dir.join(file))?;
        }
        for entry in fs::read_dir(self.root.join("licenses"))? {
            let entry = entry?;
            // The standard texts are in the notices, where they are needed.
            if entry.file_type()?.is_file() {
                copy(&entry.path(), &dir.join("licenses").join(entry.file_name()))?;
            }
        }
        fs::write(dir.join(THIRD_PARTY), &self.notices)?;
        Ok(())
    }

    /// `packaging/README.txt` for `platform`; `folders` are the settings,
    /// recovery and cache folders.
    fn readme(
        &self,
        platform: &str,
        start: &str,
        needs: &str,
        folders: [&str; 3],
        remove: &str,
        licences: &str,
    ) -> Result<String> {
        let mut text = fs::read_to_string(self.root.join("packaging/README.txt"))?;
        let [settings, recovery, cache] = folders;
        for (field, value) in [
            ("@VERSION@", self.version.as_str()),
            ("@PLATFORM@", platform),
            ("@START@", start),
            ("@NEEDS@", needs),
            ("@SETTINGS@", settings),
            ("@RECOVERY@", recovery),
            ("@CACHE@", cache),
            ("@REMOVE@", remove),
            ("@LICENCES@", licences),
        ] {
            text = text.replace(field, value);
        }
        if text.contains(self.commit) && !text.contains('@') {
            Ok(text)
        } else {
            Err("packaging/README.txt has a field the package does not fill".into())
        }
    }
}

/// Copy `from` to `to`, its folders made.
fn copy(from: &Path, to: &Path) -> Result<()> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(from, to).map_err(|e| format!("{}: {e}", from.display()))?;
    Ok(())
}

/// The newest glibc a program asks for: the highest `GLIBC_2.x` it names.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn glibc(program: &[u8]) -> Option<(u32, u32)> {
    const NAME: &[u8] = b"GLIBC_";
    program
        .windows(NAME.len())
        .enumerate()
        .filter(|(_, w)| *w == NAME)
        .filter_map(|(at, _)| {
            let rest = program.get(at + NAME.len()..)?;
            let end = rest
                .iter()
                .position(|b| !b.is_ascii_digit() && *b != b'.')?;
            let text = std::str::from_utf8(rest.get(..end)?).ok()?;
            let mut parts = text.split('.');
            Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
        })
        .max()
}

/// `plist` with the version and the build number.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn versioned(plist: &str, version: &str, build: &str) -> Result<String> {
    let mut out = plist.to_owned();
    for (key, value) in [
        ("CFBundleShortVersionString", version),
        ("CFBundleVersion", build),
    ] {
        let tag = format!("<key>{key}</key>");
        let at = out
            .find(&tag)
            .ok_or_else(|| format!("Info.plist has no {key}"))?;
        let open = at + out[at..].find("<string>").ok_or("a key without a string")? + 8;
        let close = open + out[open..].find("</string>").ok_or("an open string")?;
        out.replace_range(open..close, value);
    }
    Ok(out)
}

/// `dir` and what is in it as one zip.
#[cfg(windows)]
fn zip(dir: &Path, archive: &Path) -> Result<PathBuf> {
    use std::io::Write;
    let mut zip = zip::ZipWriter::new(fs::File::create(archive)?);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::default());
    let parent = dir.parent().ok_or("no folder to pack")?;
    let mut folders = vec![dir.to_path_buf()];
    while let Some(folder) = folders.pop() {
        for entry in fs::read_dir(&folder)? {
            let path = entry?.path();
            if path.is_dir() {
                folders.push(path);
                continue;
            }
            let name = path
                .strip_prefix(parent)?
                .to_string_lossy()
                .replace('\\', "/");
            zip.start_file(name, options)?;
            zip.write_all(&fs::read(&path)?)?;
        }
    }
    zip.finish()?;
    Ok(archive.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_newest_glibc_is_read() {
        let program = b"\0GLIBC_2.2.5\0GLIBC_2.34\0GLIBC_2.9\0GLIBC_PRIVATE\0";
        assert_eq!(glibc(program), Some((2, 34)));
        assert_eq!(glibc(b"static"), None);
    }

    /// The bundle's own Info.plist takes the version and the build.
    #[test]
    fn the_plist_is_versioned() -> Result<()> {
        let plist = fs::read_to_string(root()?.join("apps/sparksmith/macos/Info.plist"))?;
        let out = versioned(&plist, "1.2.3", "456")?;
        for value in ["<string>1.2.3</string>", "<string>456</string>"] {
            assert!(out.contains(value), "{value}");
        }
        assert_eq!(out.matches("<key>").count(), plist.matches("<key>").count());
        Ok(())
    }

    /// The menu entry, the file type and the app agree on their names.
    #[test]
    fn the_linux_files_take_the_product_names() -> Result<()> {
        let id = product::BUNDLE_ID;
        let dir = root()?.join("packaging/linux");
        let entry = fs::read_to_string(dir.join(format!("{id}.desktop")))?;
        for line in [
            format!("Name={}", product::NAME),
            format!("Icon={id}"),
            format!("StartupWMClass={id}"),
            "MimeType=application/x-sparksmith;".to_owned(),
        ] {
            assert!(entry.lines().any(|l| l == line), "{line}");
        }
        let types = fs::read_to_string(dir.join(format!("{id}.xml")))?;
        assert!(types.contains("type=\"application/x-sparksmith\""));
        assert!(types.contains("pattern=\"*.sparksmith\""));
        Ok(())
    }
}
