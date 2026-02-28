use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use std::io::{BufRead, Read};

use crate::*;

/// The `--queryformat` string used to query RPM. This is the format that
/// `load_from_str` and `load_from_reader` expect.
///
/// The `\t` and `\n` here are literal backslash escapes for rpm to interpret
/// (raw strings pass them through without Rust processing them).
pub(crate) const QUERYFORMAT: &str = concat!(
    // Per-package header line:
    r"@@PKG@@\t%{NAME}\t%{VERSION}\t%{RELEASE}\t%{EPOCH}\t%{ARCH}",
    r"\t%{LICENSE}\t%{SIZE}\t%{BUILDTIME}\t%{INSTALLTIME}",
    r"\t%{SOURCERPM}\t%{FILEDIGESTALGO}\n",
    // Per-file lines (iterated with []):
    r"[@@FILE@@\t%{FILENAMES}\t%{FILESIZES}\t%{FILEMODES}\t%{FILEMTIMES}",
    r"\t%{FILEDIGESTS}\t%{FILEFLAGS}",
    r"\t%{FILEUSERNAME}\t%{FILEGROUPNAME}\t%{FILELINKTOS}\n]",
    // Per-changelog lines (iterated with []):
    r"[@@CL@@\t%{CHANGELOGTIME}\n]",
);

/// Expected number of tab-separated fields after stripping the @@PKG@@ prefix.
const PKG_FIELDS: usize = 11;
/// Expected number of tab-separated fields after stripping the @@FILE@@ prefix.
const FILE_FIELDS: usize = 9;

/// Stream-parse queryformat output from a reader.
pub(crate) fn load_from_reader_impl<R: Read>(reader: R) -> Result<Packages> {
    let mut packages = Packages::new();
    let mut current_pkg: Option<Package> = None;
    // Whether the current package is gpg-pubkey (skip its FILE/CL lines).
    let mut skip = false;

    for (line_no, line) in std::io::BufReader::new(reader).lines().enumerate() {
        let line = line.context("reading line")?;
        if line.is_empty() {
            continue;
        }

        if let Some(rest) = line.strip_prefix("@@PKG@@\t") {
            // Finalize previous package.
            if let Some(pkg) = current_pkg.take() {
                packages.insert(pkg.name.clone(), pkg);
            }

            let fields: Vec<&str> = rest.split('\t').collect();
            if fields.len() != PKG_FIELDS {
                bail!(
                    "line {}: expected {PKG_FIELDS} fields in PKG line, got {}",
                    line_no + 1,
                    fields.len()
                );
            }

            let name = fields[0];
            // Skip gpg-pubkey entries (they lack Arch and aren't real packages).
            if name == "gpg-pubkey" {
                skip = true;
                continue;
            }

            skip = false;
            let pkg = parse_pkg_header(&fields)
                .with_context(|| format!("parsing package header at line {}", line_no + 1))?;
            current_pkg = Some(pkg);
        } else if skip {
            // Consume FILE/CL lines for skipped packages.
            continue;
        } else if let Some(rest) = line.strip_prefix("@@FILE@@\t") {
            let pkg = current_pkg
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("line {}: FILE line before any PKG", line_no + 1))?;
            let fields: Vec<&str> = rest.split('\t').collect();
            if fields.len() != FILE_FIELDS {
                bail!(
                    "line {}: expected {} fields in FILE line for '{}', got {}",
                    line_no + 1,
                    FILE_FIELDS,
                    pkg.name,
                    fields.len()
                );
            }
            let (path, info) = parse_file_line(&fields)
                .with_context(|| format!("line {}: file in '{}'", line_no + 1, pkg.name))?;
            pkg.files.insert(path, info);
        } else if let Some(rest) = line.strip_prefix("@@CL@@\t") {
            let pkg = current_pkg
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("line {}: CL line before any PKG", line_no + 1))?;
            let time: u64 = rest.parse().with_context(|| {
                format!(
                    "line {}: invalid changelog time for '{}'",
                    line_no + 1,
                    pkg.name
                )
            })?;
            pkg.changelog_times.push(time);
        } else {
            bail!(
                "line {}: unexpected line format: {}",
                line_no + 1,
                &line[..line.len().min(80)]
            );
        }
    }

    // Finalize last package.
    if let Some(pkg) = current_pkg.take() {
        packages.insert(pkg.name.clone(), pkg);
    }

    Ok(packages)
}

/// Parse queryformat output from a string.
pub(crate) fn load_from_str_impl(input: &str) -> Result<Packages> {
    load_from_reader_impl(input.as_bytes())
}

/// Parse the package header fields from a @@PKG@@ line into a partially-built
/// Package (files and changelog_times are filled in later).
fn parse_pkg_header(fields: &[&str]) -> Result<Package> {
    assert_eq!(fields.len(), PKG_FIELDS); // checked by caller
    let name = fields[0];
    let epoch = match parse_optional(fields[3]) {
        None => None,
        Some(s) => Some(
            s.parse::<u32>()
                .with_context(|| format!("{name}: invalid epoch '{s}'"))?,
        ),
    };
    let arch = parse_optional(fields[4])
        .ok_or_else(|| anyhow::anyhow!("{name}: missing arch"))?
        .to_string();
    let size = fields[6]
        .parse::<u64>()
        .with_context(|| format!("{name}: invalid size"))?;
    let buildtime = fields[7]
        .parse::<u64>()
        .with_context(|| format!("{name}: invalid buildtime"))?;
    let installtime = fields[8]
        .parse::<u64>()
        .with_context(|| format!("{name}: invalid installtime"))?;
    let sourcerpm = parse_optional(fields[9]).map(|s| s.to_string());

    let digest_algo = match parse_optional(fields[10]) {
        None => None,
        Some(s) => {
            let v = s
                .parse::<u32>()
                .with_context(|| format!("{name}: invalid filedigestalgo '{s}'"))?;
            Some(
                DigestAlgorithm::try_from(v)
                    .map_err(|_| anyhow::anyhow!("{name}: unknown digest algorithm {v}"))?,
            )
        }
    };

    Ok(Package {
        name: name.to_string(),
        version: fields[1].to_string(),
        release: fields[2].to_string(),
        epoch,
        arch,
        license: fields[5].to_string(),
        size,
        buildtime,
        installtime,
        sourcerpm,
        digest_algo,
        changelog_times: Vec::new(),
        files: Files::new(),
    })
}

/// Map the RPM `(none)` sentinel to `None`.
fn parse_optional(s: &str) -> Option<&str> {
    if s == "(none)" { None } else { Some(s) }
}

impl TryFrom<u32> for DigestAlgorithm {
    type Error = ();

    fn try_from(v: u32) -> Result<Self, Self::Error> {
        match v {
            x if x == Self::Md5 as u32 => Ok(Self::Md5),
            x if x == Self::Sha1 as u32 => Ok(Self::Sha1),
            x if x == Self::RipeMd160 as u32 => Ok(Self::RipeMd160),
            x if x == Self::Md2 as u32 => Ok(Self::Md2),
            x if x == Self::Tiger192 as u32 => Ok(Self::Tiger192),
            x if x == Self::Haval5160 as u32 => Ok(Self::Haval5160),
            x if x == Self::Sha256 as u32 => Ok(Self::Sha256),
            x if x == Self::Sha384 as u32 => Ok(Self::Sha384),
            x if x == Self::Sha512 as u32 => Ok(Self::Sha512),
            x if x == Self::Sha224 as u32 => Ok(Self::Sha224),
            x if x == Self::Sha3_256 as u32 => Ok(Self::Sha3_256),
            x if x == Self::Sha3_512 as u32 => Ok(Self::Sha3_512),
            _ => Err(()),
        }
    }
}

/// Parse a @@FILE@@ line and return the path and file info.
fn parse_file_line(fields: &[&str]) -> Result<(Utf8PathBuf, FileInfo)> {
    assert_eq!(fields.len(), FILE_FIELDS); // checked by caller
    let path = Utf8Path::new(fields[0]);
    let size = fields[1]
        .parse::<u64>()
        .with_context(|| format!("invalid filesize for {path}"))?;
    let mode = fields[2]
        .parse::<u16>()
        .with_context(|| format!("invalid filemode for {path}"))?;
    let mtime = fields[3]
        .parse::<u64>()
        .with_context(|| format!("invalid filemtime for {path}"))?;
    let digest = if fields[4].is_empty() {
        None
    } else {
        Some(fields[4].to_string())
    };
    let flags = fields[5]
        .parse::<u32>()
        .with_context(|| format!("invalid fileflags for {path}"))?;
    let linkto = if fields[8].is_empty() {
        None
    } else {
        Some(Utf8PathBuf::from(fields[8]))
    };

    let info = FileInfo {
        size,
        mode,
        mtime,
        digest,
        flags: FileFlags::from_raw(flags),
        user: fields[6].to_string(),
        group: fields[7].to_string(),
        linkto,
    };

    Ok((path.to_path_buf(), info))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_pkg_line(name: &str) -> String {
        format!(
            "@@PKG@@\t{name}\t1.0\t1.fc42\t(none)\tx86_64\tMIT\t100\t1000\t2000\tfoo.src.rpm\t8\n"
        )
    }

    fn make_file_line(path: &str) -> String {
        format!("@@FILE@@\t{path}\t100\t33188\t1000\taabbccdd\t0\troot\troot\t\n")
    }

    #[test]
    fn test_empty_input() {
        let packages = load_from_str_impl("").unwrap();
        assert!(packages.is_empty());
    }

    #[test]
    fn test_gpg_pubkey_skipped() {
        let input =
            "@@PKG@@\tgpg-pubkey\t1.0\t1.fc42\t(none)\t(none)\tpubkey\t0\t0\t0\t(none)\t(none)\n";
        let packages = load_from_str_impl(input).unwrap();
        assert!(packages.is_empty());
    }

    #[test]
    fn test_none_optional_fields() {
        let input = "@@PKG@@\ttest\t1.0\t1\t(none)\tx86_64\tMIT\t0\t0\t0\t(none)\t(none)\n";
        let packages = load_from_str_impl(input).unwrap();
        assert_eq!(packages["test"].epoch, None);
        assert_eq!(packages["test"].sourcerpm, None);
        assert_eq!(packages["test"].digest_algo, None);
        assert!(packages["test"].files.is_empty());
        assert!(packages["test"].changelog_times.is_empty());
    }

    #[test]
    fn test_no_files_with_changelog() {
        let mut input = make_pkg_line("test");
        input.push_str("@@CL@@\t3000\n");
        input.push_str("@@CL@@\t2000\n");
        input.push_str("@@CL@@\t1000\n");
        let packages = load_from_str_impl(&input).unwrap();
        assert!(packages["test"].files.is_empty());
        assert_eq!(packages["test"].changelog_times, vec![3000, 2000, 1000]);
    }

    #[test]
    fn test_with_files_no_changelog() {
        let mut input = make_pkg_line("test");
        input.push_str(&make_file_line("/usr/bin/foo"));
        input.push_str(&make_file_line("/usr/bin/bar"));
        let packages = load_from_str_impl(&input).unwrap();
        assert_eq!(packages["test"].files.len(), 2);
        assert!(packages["test"].changelog_times.is_empty());
    }

    #[test]
    fn test_with_files_and_changelog() {
        let mut input = make_pkg_line("test");
        input.push_str(&make_file_line("/usr/bin/foo"));
        input.push_str(&make_file_line("/usr/bin/bar"));
        input.push_str("@@CL@@\t2000\n");
        input.push_str("@@CL@@\t1000\n");
        let packages = load_from_str_impl(&input).unwrap();
        assert_eq!(packages["test"].files.len(), 2);
        assert_eq!(packages["test"].changelog_times, vec![2000, 1000]);
    }

    #[test]
    fn test_multiple_packages() {
        let mut input = make_pkg_line("alpha");
        input.push_str(&make_file_line("/usr/bin/alpha"));
        input.push_str(&make_pkg_line("beta"));
        input.push_str(&make_file_line("/usr/bin/beta"));
        let packages = load_from_str_impl(&input).unwrap();
        assert_eq!(packages.len(), 2);
        assert!(packages.contains_key("alpha"));
        assert!(packages.contains_key("beta"));
    }

    #[test]
    fn test_error_conditions() {
        // Wrong number of fields in PKG line.
        assert!(load_from_str_impl("@@PKG@@\tfoo\t1.0\n").is_err());

        // Wrong number of fields in FILE line.
        let mut input = make_pkg_line("test");
        input.push_str("@@FILE@@\t/a\t0\n");
        assert!(load_from_str_impl(&input).is_err());

        // FILE line before any PKG line.
        assert!(load_from_str_impl("@@FILE@@\t/a\t0\t33188\t0\t\t0\troot\troot\t\n").is_err());

        // Unrecognized line format.
        assert!(load_from_str_impl("garbage\n").is_err());
    }

    #[test]
    fn test_symlink_and_empty_digest() {
        let mut input = make_pkg_line("test");
        // A symlink with empty digest
        input.push_str("@@FILE@@\t/usr/bin/sh\t4\t41471\t1000\t\t0\troot\troot\tbash\n");
        let packages = load_from_str_impl(&input).unwrap();
        let sh = &packages["test"].files[Utf8Path::new("/usr/bin/sh")];
        assert!(sh.digest.is_none());
        assert_eq!(sh.linkto.as_deref(), Some(Utf8Path::new("bash")));
    }
}
