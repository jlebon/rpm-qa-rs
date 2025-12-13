use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use serde::Deserialize;
use serde_json::Deserializer;
use std::io::Read;

use crate::*;

/// Helper to deserialize a value that can be either a single item or an array.
/// rpm outputs scalars for single-file packages but arrays for multi-file packages.
fn deserialize_one_or_many<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany<T> {
        One(T),
        Many(Vec<T>),
    }

    match Option::<OneOrMany<T>>::deserialize(deserializer)? {
        None => Ok(Vec::new()),
        Some(OneOrMany::One(v)) => Ok(vec![v]),
        Some(OneOrMany::Many(v)) => Ok(v),
    }
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
    #[serde(default, deserialize_with = "deserialize_one_or_many")]
    basenames: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_one_or_many")]
    dirnames: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_one_or_many")]
    dirindexes: Vec<u32>,
    #[serde(default, deserialize_with = "deserialize_one_or_many")]
    filesizes: Vec<u64>,
    #[serde(default, deserialize_with = "deserialize_one_or_many")]
    filemodes: Vec<u16>,
    #[serde(default, deserialize_with = "deserialize_one_or_many")]
    filemtimes: Vec<u64>,
    #[serde(default, deserialize_with = "deserialize_one_or_many")]
    filedigests: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_one_or_many")]
    fileflags: Vec<u32>,
    #[serde(default, deserialize_with = "deserialize_one_or_many")]
    fileusername: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_one_or_many")]
    filegroupname: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_one_or_many")]
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
            let path = Utf8Path::new(dirname).join(basename);

            let size = *raw.filesizes.get(i).ok_or_else(|| {
                anyhow::anyhow!("{}: missing filesize for {} (index {})", raw.name, path, i)
            })?;
            let mode = *raw.filemodes.get(i).ok_or_else(|| {
                anyhow::anyhow!("{}: missing filemode for {} (index {})", raw.name, path, i)
            })?;
            let mtime = *raw.filemtimes.get(i).ok_or_else(|| {
                anyhow::anyhow!("{}: missing filemtime for {} (index {})", raw.name, path, i)
            })?;
            let flags = *raw.fileflags.get(i).ok_or_else(|| {
                anyhow::anyhow!("{}: missing fileflags for {} (index {})", raw.name, path, i)
            })?;
            let user = raw.fileusername.get(i).ok_or_else(|| {
                anyhow::anyhow!(
                    "{}: missing fileusername for {} (index {})",
                    raw.name,
                    path,
                    i
                )
            })?;
            let group = raw.filegroupname.get(i).ok_or_else(|| {
                anyhow::anyhow!(
                    "{}: missing filegroupname for {} (index {})",
                    raw.name,
                    path,
                    i
                )
            })?;
            let filedigest = raw.filedigests.get(i).ok_or_else(|| {
                anyhow::anyhow!(
                    "{}: missing filedigest for {} (index {})",
                    raw.name,
                    path,
                    i
                )
            })?;
            let linkto = raw.filelinktos.get(i).ok_or_else(|| {
                anyhow::anyhow!(
                    "{}: missing filelinkto for {} (index {})",
                    raw.name,
                    path,
                    i
                )
            })?;

            // Build digest if non-empty
            let digest = if filedigest.is_empty() {
                None
            } else {
                digest_algo.map(|algorithm| FileDigest {
                    algorithm,
                    hex: filedigest.clone(),
                })
            };

            // Build linkto if non-empty
            let linkto = if linkto.is_empty() {
                None
            } else {
                Some(Utf8PathBuf::from(linkto))
            };

            let info = FileInfo {
                size,
                mode,
                mtime,
                digest,
                flags: FileFlags::from_raw(flags),
                user: user.clone(),
                group: group.clone(),
                linkto,
            };

            files.insert(path, info);
        }

        let arch = raw
            .arch
            .ok_or_else(|| anyhow::anyhow!("{}: missing arch", raw.name))?;

        Ok(Package {
            name: raw.name,
            version: raw.version,
            release: raw.release,
            epoch: raw.epoch,
            arch,
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
        let raw = result.context("parsing JSON")?;
        // Skip gpg-pubkey entries (they lack Arch and aren't real packages)
        if raw.name == "gpg-pubkey" {
            continue;
        }
        let pkg = Package::try_from(raw).context("converting raw package")?;
        packages.insert(pkg.name.clone(), pkg);
    }
    Ok(packages)
}
