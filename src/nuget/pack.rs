use std::io::{copy, Cursor, Error as IoError, Seek, Write};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::borrow::Cow;
use std::collections::HashMap;
use zip::CompressionMethod;
use zip::write::{FileOptions, ZipWriter};
use zip::result::ZipError;

use super::Buf;
use super::util::{openxml, xml};
use args::Target;

/// Args for building a `nupkg` with potentially multiple targets.
#[derive(Debug, PartialEq)]
pub struct NugetPackArgs<'a> {
    pub id: Cow<'a, str>,
    pub version: Cow<'a, str>,
    pub spec: &'a Buf,
    pub cargo_libs: HashMap<Target, Cow<'a, Path>>,
}

/// A formatted `nupkg`.
#[derive(Debug, PartialEq)]
pub struct Nupkg<'a> {
    pub name: Cow<'a, str>,
    pub rids: Vec<Cow<'a, str>>,
    pub buf: Buf,
}

fn options() -> FileOptions {
    FileOptions::default().compression_method(CompressionMethod::Deflated)
}

/// Pack a `nuspec` and native libs into a `nupkg`.
pub fn pack<'a>(args: NugetPackArgs<'a>) -> Result<Nupkg, NugetPackError> {
    let pkgs: Vec<_> = args.cargo_libs
        .iter()
        .filter_map(|(target, path)| {
            if target.is_unknown() {
                None
            } else {
                Some((target.rid(), path))
            }
        })
        .collect();

    if pkgs.len() == 0 {
        Err(NugetPackError::NoValidTargets)?
    }

    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));

    let nuspec_path = {
        let mut path = PathBuf::new();
        path.set_file_name(args.id.as_ref());
        path.set_extension("nuspec");

        path
    };

    write_rels(&mut writer, &nuspec_path)?;
    write_content_types(&mut writer)?;

    writer.start_file(nuspec_path.to_string_lossy(), options())?;
    writer.write_all(&args.spec)?;

    for &(ref rid, ref lib_path) in &pkgs {
        write_lib(&mut writer, &args.id, rid, lib_path).map_err(|e| {
            NugetPackError::WriteLib {
                rid: rid.to_string(),
                lib_path: lib_path.to_string_lossy().into_owned(),
                err: e,
            }
        })?;
    }

    let buf = writer.finish()?.into_inner();

    let rids = pkgs.into_iter().map(|(rid, _)| rid).collect();
    let name = format!("{}.{}.nupkg", args.id, args.version);

    Ok(Nupkg {
        name: name.into(),
        rids: rids,
        buf: buf.into(),
    })
}

/// Write `/runtimes/{rid}/native/{lib}`.
fn write_lib<W>(
    writer: &mut ZipWriter<W>,
    id: &str,
    rid: &str,
    lib_path: &Path,
) -> Result<(), NugetWriteLibError>
where
    W: Write + Seek,
{
    let mut path = PathBuf::new();
    path.push("runtimes");
    path.push(rid);
    path.push("native");
    path.push(id);

    if let Some(extension) = lib_path.extension() {
        path.set_extension(extension);
    }

    writer.start_file(path.to_string_lossy(), options())?;

    let mut lib = File::open(lib_path)?;
    copy(&mut lib, writer)?;

    Ok(())
}

/// Write `/_rels/.rels`.
fn write_rels<W>(writer: &mut ZipWriter<W>, nuspec_path: &Path) -> Result<(), NugetPackError>
where
    W: Write + Seek,
{
    let (path, xml) = openxml::relationships(&nuspec_path)?;

    writer.start_file(path.to_string_lossy(), options())?;
    writer.write_all(&xml)?;

    Ok(())
}

/// Write `/[Content_Types].xml`.
fn write_content_types<W>(writer: &mut ZipWriter<W>) -> Result<(), NugetPackError>
where
    W: Write + Seek,
{
    let (path, xml) = openxml::content_types()?;

    writer.start_file(path.to_string_lossy(), options())?;
    writer.write_all(&xml)?;

    Ok(())
}

quick_error!{
    #[derive(Debug)]
    pub enum NugetPackError {
        /// No valid platform targets were available
        NoValidTargets {
            display("No valid platform targets were supplied\nThis probably means you're running on an unsupported platform")
        }
        /// A zip writing error.
        Zip(err: ZipError) {
            display("Error building nupkg\nCaused by: {}", err)
            from()
        }
        /// A general io error.
        Io(err: IoError) {
            display("Error building nupkg\nCaused by: {}", err)
            from()
        }
        /// An xml formatting error.
        Xml(err: xml::Error) {
            display("Error building nupkg\nCaused by: {}", err)
            from()
        }
        /// An error with a specific library.
        WriteLib { rid: String, lib_path: String, err: NugetWriteLibError } {
            display("Error reading lib {} at path {}\nCaused by: {}", rid, lib_path, err)
        }
    }
}

quick_error!{
    #[derive(Debug)]
    pub enum NugetWriteLibError {
        /// A zip writing error.
        Zip(err: ZipError) {
            display("Error reading lib\nCaused by: {}", err)
            from()
        }
        /// A general io error.
        Io(err: IoError) {
            display("Error reading lib\nCaused by: {}", err)
            from()
        }
        /// An error parsing a library path.
        BadPath { path: String } {
            display("Error parsing path '{}'", path)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::collections::HashMap;
    use super::*;

    macro_rules! assert_inavlid {
        ($args:ident, $err:pat) => ({
            let nuspec = pack($args);

            match nuspec {
                Err($err) => (),
                r => panic!("{:?}", r)
            }
        })
    }

    #[test]
    fn pack_with_no_targets() {
        let args = NugetPackArgs {
            id: "some_pkg".into(),
            version: "0.1.1".into(),
            spec: &vec![].into(),
            cargo_libs: HashMap::new(),
        };

        assert_inavlid!(args, NugetPackError::NoValidTargets);
    }

    #[test]
    fn pack_with_unknown_target() {
        let mut targets = HashMap::new();
        targets.insert(Target::Unknown, PathBuf::new().into());

        let args = NugetPackArgs {
            id: "some_pkg".into(),
            version: "0.1.1".into(),
            spec: &vec![].into(),
            cargo_libs: targets,
        };

        assert_inavlid!(args, NugetPackError::NoValidTargets);
    }
}
