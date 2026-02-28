# rpm-qa-rs

A thin Rust wrapper around `rpm -qa`.

This crate provides functions to load and parse the output from
`rpm -qa --queryformat`, returning package metadata as a map of package names to
`Package` structs. RPM 6.0+ supports `--json` but (1) that's actually _slower_
than `--queryformat`, and (2) we want compatibility with older RPM.

## Usage

```rust
use rpm_qa::{load, load_from_rootfs};
use std::path::Path;

// Query the host system
let packages = load().unwrap();

// Query a different rootfs
let packages = load_from_rootfs(Path::new("/mnt/sysroot")).unwrap();

// Access package metadata
for (name, pkg) in &packages {
    println!("{}-{}-{}", name, pkg.version, pkg.release);
}
```

## Comparison with librpm.rs

The [librpm.rs](https://github.com/rpm-software-management/librpm.rs) project
provides Rust FFI bindings for librpm. For anything tightly integrated into
RPM, it would be great for you to contribute there and use that instead. For
example, this allows cheaper queries that don't load things like changelogs.
And the scope of that project is to eventually cover other major parts of the
RPM API such as building and signing. This project OTOH is limited just to rpmdb
querying. Some of the reasons why I built it were:
- The librpm.rs bindings are currently limited and don't for example support RPM
file listings (though this likely wouldn't be hard to add).
- For my use case of this library, I want to avoid linking to other libraries
to make the final binary portable. Ideally, rpm is just _one_ backend possible.
There's Rust feature flags of course, but that implies compiling multiple
binaries/packaging work.
- For my use case of this library, I'm not concerned with taking 1 or 2 seconds
longer to load the whole rpmdb and throw away e.g. changelog entries.
- It's unclear how much attention is paid to librpm.rs today. crates.io shows
no dependents and limited activity overall. It felt safer (in both the Rust and
non-Rust sense) to just use the `rpm` CLI instead.
