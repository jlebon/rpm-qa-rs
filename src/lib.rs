//! A thin Rust wrapper around `rpm -qa --json`
//!
//! This crate provides functions to load and parse the JSON output from
//! `rpm -qa --json`, returning package metadata as a map of package names
//! to `Package` structs.

mod raw;

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use std::collections::{BTreeMap, HashMap};
use std::io::Read;
use std::process::Command;

/// A map of package names to their metadata.
pub type Packages = HashMap<String, Package>;

/// A map of file paths to their metadata.
pub type Files = BTreeMap<Utf8PathBuf, FileInfo>;

/// Cryptographic hash algorithm used for file digests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DigestAlgorithm {
    /// MD5 (legacy, insecure).
    Md5 = 1,
    /// SHA-1 (legacy, insecure).
    Sha1 = 2,
    /// RIPEMD-160.
    RipeMd160 = 3,
    /// MD2 (obsolete).
    Md2 = 5,
    /// TIGER-192.
    Tiger192 = 6,
    /// HAVAL-5-160.
    Haval5160 = 7,
    /// SHA-256 (current default).
    Sha256 = 8,
    /// SHA-384.
    Sha384 = 9,
    /// SHA-512.
    Sha512 = 10,
    /// SHA-224.
    Sha224 = 11,
    /// SHA3-256.
    Sha3_256 = 12,
    /// SHA3-512.
    Sha3_512 = 14,
}

/// A file digest with its algorithm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDigest {
    /// The hash algorithm used.
    pub algorithm: DigestAlgorithm,
    /// The hex-encoded digest value.
    pub hex: String,
}

/// File attribute flags from the RPM spec file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FileFlags(u32);

impl FileFlags {
    /// File is a configuration file (`%config`).
    pub const CONFIG: u32 = 1 << 0;
    /// File is documentation (`%doc`).
    pub const DOC: u32 = 1 << 1;
    /// Missing file is OK (`%config(missingok)`).
    pub const MISSINGOK: u32 = 1 << 3;
    /// Don't replace existing file (`%config(noreplace)`).
    pub const NOREPLACE: u32 = 1 << 4;
    /// File is a ghost (`%ghost`).
    pub const GHOST: u32 = 1 << 6;
    /// File is a license (`%license`).
    pub const LICENSE: u32 = 1 << 7;
    /// File is a README (`%readme`).
    pub const README: u32 = 1 << 8;
    /// File is a build artifact (`%artifact`).
    pub const ARTIFACT: u32 = 1 << 12;

    /// Create from raw flag value.
    pub fn from_raw(value: u32) -> Self {
        Self(value)
    }

    /// Get the raw flag value.
    pub fn raw(&self) -> u32 {
        self.0
    }

    /// Check if the config flag is set.
    pub fn is_config(&self) -> bool {
        self.0 & Self::CONFIG != 0
    }

    /// Check if the doc flag is set.
    pub fn is_doc(&self) -> bool {
        self.0 & Self::DOC != 0
    }

    /// Check if the missingok flag is set.
    pub fn is_missingok(&self) -> bool {
        self.0 & Self::MISSINGOK != 0
    }

    /// Check if the noreplace flag is set.
    pub fn is_noreplace(&self) -> bool {
        self.0 & Self::NOREPLACE != 0
    }

    /// Check if the ghost flag is set.
    pub fn is_ghost(&self) -> bool {
        self.0 & Self::GHOST != 0
    }

    /// Check if the license flag is set.
    pub fn is_license(&self) -> bool {
        self.0 & Self::LICENSE != 0
    }

    /// Check if the readme flag is set.
    pub fn is_readme(&self) -> bool {
        self.0 & Self::README != 0
    }

    /// Check if the artifact flag is set.
    pub fn is_artifact(&self) -> bool {
        self.0 & Self::ARTIFACT != 0
    }
}

/// Metadata for a file contained in an RPM package.
#[derive(Debug, Clone)]
pub struct FileInfo {
    /// File size in bytes.
    pub size: u64,
    /// Unix file mode (permissions and type).
    pub mode: u16,
    /// Unix modification timestamp.
    pub mtime: u64,
    /// File digest, if present (directories and symlinks have none).
    pub digest: Option<FileDigest>,
    /// File attribute flags.
    pub flags: FileFlags,
    /// Owner username.
    pub user: String,
    /// Owner group name.
    pub group: String,
    /// Symlink target, if this is a symbolic link.
    pub linkto: Option<Utf8PathBuf>,
}

/// Metadata for an installed RPM package.
#[derive(Debug)]
pub struct Package {
    /// Package name.
    pub name: String,
    /// Package version.
    pub version: String,
    /// Package release.
    pub release: String,
    /// Package epoch, if present.
    pub epoch: Option<u32>,
    /// The architecture the package is for. `noarch` is a special case denoting
    /// an architecture independent package.
    pub arch: String,
    /// License of the package contents.
    pub license: String,
    /// Installed package size.
    pub size: u64,
    /// Unix timestamp of package build time.
    pub buildtime: u64,
    /// Unix timestamp of package installation.
    pub installtime: u64,
    /// Package source rpm file name.
    pub sourcerpm: Option<String>,
    /// Files contained in this package.
    pub files: Files,
}

/// Load packages from a reader containing JSON output from `rpm -qa --json`.
pub fn load_from_reader<R: Read>(reader: R) -> Result<Packages> {
    raw::load_from_reader_impl(reader)
}

/// Load packages from a string containing JSON output from `rpm -qa --json`.
pub fn load_from_str(s: &str) -> Result<Packages> {
    load_from_reader(s.as_bytes())
}

/// Load all installed RPM packages from a rootfs by running `rpm -qa --json --root`.
pub fn load_from_rootfs(rootfs: &Utf8Path) -> Result<Packages> {
    let output = Command::new("rpm")
        .args(["--root"])
        .arg(rootfs)
        .args(["-qa", "--json"])
        .output()
        .context("failed to run rpm")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        match output.status.code() {
            Some(code) => bail!("rpm command failed (exit code {}): {}", code, stderr),
            None => bail!("rpm command failed: {}", stderr),
        }
    }

    load_from_reader(output.stdout.as_slice())
}

/// Load all installed RPM packages by running `rpm -qa --json`.
pub fn load() -> Result<Packages> {
    load_from_rootfs(Utf8Path::new("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../tests/fixtures/fedora.json");

    #[test]
    fn test_load_from_str() {
        let packages = load_from_str(FIXTURE).expect("failed to load packages");
        assert!(!packages.is_empty(), "expected at least one package");

        for (name, pkg) in &packages {
            assert_eq!(name, &pkg.name);
            assert!(!pkg.version.is_empty());
            assert!(!pkg.arch.is_empty());
        }

        // Check specific packages from fixture
        assert!(packages.contains_key("glibc"));
        assert!(packages.contains_key("bash"));
        assert!(packages.contains_key("coreutils"));

        // bash has no epoch
        assert_eq!(packages["bash"].epoch, None);
        // shadow-utils has epoch 2
        assert_eq!(packages["shadow-utils"].epoch, Some(2));
        // perl-POSIX has explicit epoch 0
        assert_eq!(packages["perl-POSIX"].epoch, Some(0));
    }

    #[test]
    fn test_load_from_reader() {
        let packages = load_from_reader(FIXTURE.as_bytes()).expect("failed to load packages");
        assert!(!packages.is_empty(), "expected at least one package");
        assert!(packages.get("rpm").is_some());
    }

    #[test]
    fn test_file_parsing() {
        let packages = load_from_str(FIXTURE).expect("failed to load packages");
        let bash = packages.get("bash").expect("bash package not found");

        // bash should have files
        assert!(!bash.files.is_empty(), "bash should have files");

        // Check /usr/bin/bash exists
        let bash_bin = bash
            .files
            .get(Utf8Path::new("/usr/bin/bash"))
            .expect("/usr/bin/bash not found");
        assert!(bash_bin.size > 0, "bash binary should have non-zero size");
        assert!(bash_bin.digest.is_some(), "bash binary should have digest");
        assert_eq!(
            bash_bin.digest.as_ref().unwrap().algorithm,
            DigestAlgorithm::Sha256
        );
        assert!(
            !bash_bin.flags.is_config(),
            "bash binary is not a config file"
        );
        assert_eq!(bash_bin.user, "root");
        assert_eq!(bash_bin.group, "root");

        // Check a config file
        let bashrc = bash
            .files
            .get(Utf8Path::new("/etc/skel/.bashrc"))
            .expect("/etc/skel/.bashrc not found");
        assert!(bashrc.flags.is_config(), ".bashrc should be a config file");
        assert!(bashrc.flags.is_noreplace(), ".bashrc should be noreplace");

        // Check symlink /usr/bin/sh -> bash
        let sh = bash
            .files
            .get(Utf8Path::new("/usr/bin/sh"))
            .expect("/usr/bin/sh not found");
        assert!(sh.linkto.is_some(), "/usr/bin/sh should be a symlink");
        assert_eq!(sh.linkto.as_ref().unwrap(), "bash");

        // Check ghost files from setup package
        let setup = packages.get("setup").expect("setup package not found");

        // /run/motd is a pure ghost file (flag=64)
        let motd = setup
            .files
            .get(Utf8Path::new("/run/motd"))
            .expect("/run/motd not found");
        assert!(motd.flags.is_ghost(), "/run/motd should be a ghost");
        assert!(!motd.flags.is_config(), "/run/motd is not a config file");
        assert!(motd.digest.is_none(), "ghost files have no digest");

        // /etc/fstab is ghost+config+missingok+noreplace (flag=89)
        let fstab = setup
            .files
            .get(Utf8Path::new("/etc/fstab"))
            .expect("/etc/fstab not found");
        assert!(fstab.flags.is_ghost(), "/etc/fstab should be a ghost");
        assert!(
            fstab.flags.is_config(),
            "/etc/fstab should be a config file"
        );
        assert!(fstab.flags.is_missingok(), "/etc/fstab should be missingok");
        assert!(fstab.flags.is_noreplace(), "/etc/fstab should be noreplace");
    }

    #[test]
    fn test_directory_ownership() {
        // Test that files can be owned by a different package than the directory they reside in.
        // In this fixture:
        // - rpm owns /usr/lib/rpm/macros.d/ (the directory)
        // - fedora-release-common owns /usr/lib/rpm/macros.d/macros.dist (a file in that directory)
        let packages = load_from_str(FIXTURE).expect("failed to load packages");

        let rpm = packages.get("rpm").expect("rpm package not found");
        let fedora_release = packages
            .get("fedora-release-common")
            .expect("fedora-release-common package not found");

        // Verify rpm owns the macros.d directory
        let macros_d = rpm
            .files
            .get(Utf8Path::new("/usr/lib/rpm/macros.d"))
            .expect("/usr/lib/rpm/macros.d not found in rpm");
        // Directory mode: 0o40755 = 16877
        assert_eq!(
            macros_d.mode & 0o170000,
            0o040000,
            "macros.d should be a directory"
        );

        // Verify fedora-release-common owns macros.dist file
        assert!(
            fedora_release
                .files
                .contains_key(Utf8Path::new("/usr/lib/rpm/macros.d/macros.dist")),
            "/usr/lib/rpm/macros.d/macros.dist not found in fedora-release-common"
        );

        // Verify the file is NOT in rpm's file list
        assert!(
            rpm.files
                .get(Utf8Path::new("/usr/lib/rpm/macros.d/macros.dist"))
                .is_none(),
            "macros.dist should not be owned by rpm"
        );

        // Verify the directory is NOT in fedora-release-common's file list
        assert!(
            fedora_release
                .files
                .get(Utf8Path::new("/usr/lib/rpm/macros.d"))
                .is_none(),
            "macros.d directory should not be owned by fedora-release-common"
        );
    }
}
