use std::{
    fmt::Display,
    fs,
    io::{prelude::*, BufReader},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

use anyhow::Context;
use flate2::read::ZlibDecoder;
use flate2::{write::ZlibEncoder, Compression};
use sha1::{Digest, Sha1};

const OBJECTS_PATH: &str = ".git/objects";

#[derive(PartialEq, Clone, Debug)]
pub enum ObjectType {
    Blob,
    Tree,
    Commit,
    Tag,
    OfsDelta,
    RefDelta,
}

impl Display for ObjectType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ObjectType::Blob => write!(f, "blob"),
            ObjectType::Tree => write!(f, "tree"),
            ObjectType::Commit => write!(f, "commit"),
            ObjectType::Tag => write!(f, "tag"),
            ObjectType::OfsDelta => write!(f, "osf_delta"),
            ObjectType::RefDelta => write!(f, "ref_delta"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Header {
    pub typ: ObjectType,
    pub size: usize,
}

pub struct ObjectFile<R> {
    pub header: Header,
    pub reader: R,
}

impl ObjectFile<()> {
    pub fn read(hash: &str, custom_dir: Option<&Path>) -> anyhow::Result<ObjectFile<impl BufRead>> {
        let mut path = Self::hash_to_path(hash);
        if let Some(custom_dir) = custom_dir {
            let mut custom_path = PathBuf::from(custom_dir);
            custom_path.push(path);
            path = custom_path;
        }

        let f =
            fs::File::open(&path).with_context(|| format!("opening file {}", path.display()))?;

        let mut decoder = BufReader::new(ZlibDecoder::new(f));

        let mut buf = Vec::new();

        decoder
            .read_until(0, &mut buf)
            .context("reading object header")?;

        let header = std::ffi::CStr::from_bytes_with_nul(&buf)
            .expect("should be null terminated string")
            .to_str()
            .context("file header is not valid UTF-8")?;

        let Some((typ, size)) = header.split_once(' ') else {
            anyhow::bail!("incorrect object header: {}", header)
        };

        let size = size
            .parse::<usize>()
            .context("parsing object size in header")?;

        let object_type = match typ {
            "blob" => ObjectType::Blob,
            "tree" => ObjectType::Tree,
            "commit" => ObjectType::Commit,
            _ => anyhow::bail!("unknown object type {}", typ),
        };

        let header = Header {
            typ: object_type,
            size,
        };

        Ok(ObjectFile {
            header,
            reader: decoder,
        })
    }

    pub fn from_file(path: impl AsRef<Path>) -> anyhow::Result<ObjectFile<impl Read>> {
        let path = path.as_ref();

        let stat = fs::metadata(path).with_context(|| format!("stat file {}", path.display()))?;

        let f = fs::File::open(path).with_context(|| format!("opening file {}", path.display()))?;

        let expected_size = stat.size();
        let header = Header {
            typ: ObjectType::Blob,
            size: expected_size as usize,
        };

        let r = f.take(stat.size());

        Ok(ObjectFile { header, reader: r })
    }

    pub fn hash_to_path(hash: &str) -> PathBuf {
        let dir = &hash[..2];
        let file = &hash[2..];
        let mut path = PathBuf::from(OBJECTS_PATH);
        path.push(dir);
        path.push(file);
        path
    }
}

impl<R: Read> ObjectFile<R> {
    /// Computes and returns object hash ID
    pub fn hash(mut self) -> anyhow::Result<[u8; 20]> {
        let mut hasher = HashWriter {
            writer: std::io::sink(), // just consume all data
            hasher: Sha1::new(),
        };

        let header = self.header;
        write!(hasher, "{} {}\0", header.typ, header.size)?;

        std::io::copy(&mut self.reader, &mut hasher)
            .context("streaming object's data to hasher")?;

        let digest = hasher.hasher.finalize();

        Ok(digest.into())
    }

    /// Compresses and write object's data to disk. Returns object hash ID.
    pub fn write(&mut self, custom_dir: Option<&Path>) -> anyhow::Result<[u8; 20]> {
        let dir = tempfile::tempdir().context("creating temp dir")?;

        let tmp_file_path = dir.path().join("tmpfile");
        let tmp_file = fs::File::create(&tmp_file_path).context("creating temp file")?;

        let encoder = ZlibEncoder::new(tmp_file, Compression::fast());

        let mut compressor = HashWriter {
            writer: encoder,
            hasher: Sha1::new(),
        };

        let header = &self.header;
        write!(compressor, "{} {}\0", header.typ, header.size)?;

        std::io::copy(&mut self.reader, &mut compressor)
            .context("streaming object's content to file on disk")?;

        let _ = compressor.writer.finish()?;

        let digest = compressor.hasher.finalize();

        let hash = hex::encode(digest);
        let dir = &hash[..2]; // first 2 chars of the digest
        let filename = &hash[2..]; // rest of the digest

        let mut path = PathBuf::from(OBJECTS_PATH);
        path.push(dir);

        if let Some(custom_dir) = custom_dir {
            let mut custom_path = PathBuf::from(custom_dir);
            custom_path.push(path);
            path = custom_path;
        }

        fs::create_dir_all(&path)
            .with_context(|| format!("creating directory {}", path.display()))?;

        path.push(filename);

        fs::rename(tmp_file_path, &path)
            .with_context(|| format!("moving temp file to {}", path.display()))?;

        Ok(digest.into())
    }
}

struct HashWriter<W> {
    writer: W,
    hasher: Sha1,
}

impl<W> Write for HashWriter<W>
where
    W: Write,
{
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.writer.write(buf)?;
        self.hasher.update(&buf[..n]);
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.writer.flush()
    }
}
