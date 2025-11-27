use anyhow::{Result, bail};
use serde::Deserialize;
use serde_json::Deserializer;
use std::io::Read;

use crate::*;

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

fn deserialize_none_string<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::deserialize(deserializer)?.filter(|v| v != "(none)"))
}

/// Raw package data as deserialized from JSON (internal use).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawPackage {
    name: String,
    version: String,
    release: String,
    #[serde(default)]
    epoch: Option<u32>,
    #[serde(default)]
    arch: Option<String>,
    license: String,
    size: u64,
    buildtime: u64,
    installtime: u64,
    #[serde(deserialize_with = "deserialize_none_string")]
    sourcerpm: Option<String>,
    // File-related arrays
    #[serde(default)]
    basenames: Vec<String>,
    #[serde(default)]
    dirnames: Vec<String>,
    #[serde(default)]
    dirindexes: Vec<u32>,
    #[serde(default)]
    filesizes: Vec<u64>,
    #[serde(default)]
    filemodes: Vec<u16>,
    #[serde(default)]
    filemtimes: Vec<u64>,
    #[serde(default)]
    filedigests: Vec<String>,
    #[serde(default)]
    fileflags: Vec<u32>,
    #[serde(default)]
    fileusername: Vec<String>,
    #[serde(default)]
    filegroupname: Vec<String>,
    #[serde(default)]
    filelinktos: Vec<String>,
    #[serde(default)]
    filedigestalgo: Option<u32>,
}

impl TryFrom<RawPackage> for Package {
    type Error = anyhow::Error;

    fn try_from(raw: RawPackage) -> Result<Self> {
        let digest_algo = raw
            .filedigestalgo
            .map(|v| {
                DigestAlgorithm::try_from(v)
                    .map_err(|_| anyhow::anyhow!("{}: unknown digest algorithm {}", raw.name, v))
            })
            .transpose()?;

        let mut files = Files::new();

        for i in 0..raw.basenames.len() {
            // Reconstruct file path: dirnames[dirindexes[i]] + basenames[i]
            let Some(&dir_idx) = raw.dirindexes.get(i) else {
                bail!("{}: missing dirindex for file {}", raw.name, i);
            };
            let Some(dirname) = raw.dirnames.get(dir_idx as usize) else {
                bail!(
                    "{}: dirindex {} out of bounds for file {}",
                    raw.name,
                    dir_idx,
                    i
                );
            };
            let basename = &raw.basenames[i];
            let path = format!("{}{}", dirname, basename);

            // Build digest if present and non-empty
            let digest = raw.filedigests.get(i).and_then(|hex| {
                if hex.is_empty() {
                    None
                } else {
                    digest_algo.map(|algorithm| FileDigest {
                        algorithm,
                        hex: hex.clone(),
                    })
                }
            });

            // Build linkto if present and non-empty
            let linkto = raw
                .filelinktos
                .get(i)
                .and_then(|s| if s.is_empty() { None } else { Some(s.clone()) });

            let info = FileInfo {
                size: raw.filesizes.get(i).copied().unwrap_or(0),
                mode: raw.filemodes.get(i).copied().unwrap_or(0),
                mtime: raw.filemtimes.get(i).copied().unwrap_or(0),
                digest,
                flags: FileFlags::from_raw(raw.fileflags.get(i).copied().unwrap_or(0)),
                user: raw.fileusername.get(i).cloned().unwrap_or_default(),
                group: raw.filegroupname.get(i).cloned().unwrap_or_default(),
                linkto,
            };

            files.insert(path, info);
        }

        Ok(Package {
            name: raw.name,
            version: raw.version,
            release: raw.release,
            epoch: raw.epoch,
            arch: raw.arch.unwrap_or_default(),
            license: raw.license,
            size: raw.size,
            buildtime: raw.buildtime,
            installtime: raw.installtime,
            sourcerpm: raw.sourcerpm,
            files,
        })
    }
}

pub(crate) fn load_from_reader_impl<R: Read>(reader: R) -> Result<Packages> {
    let stream = Deserializer::from_reader(reader).into_iter::<RawPackage>();
    let mut packages = Packages::new();
    for result in stream {
        let raw = result?;
        // Skip gpg-pubkey entries (they lack Arch and aren't real packages)
        if raw.name == "gpg-pubkey" {
            continue;
        }
        let pkg = Package::try_from(raw)?;
        packages.insert(pkg.name.clone(), pkg);
    }
    Ok(packages)
}
