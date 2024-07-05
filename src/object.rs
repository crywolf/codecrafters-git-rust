use std::{
    fmt::Display,
    fs,
    io::{prelude::*, BufReader},
    path::PathBuf,
};

use anyhow::Context;
use flate2::read::ZlibDecoder;
use flate2::{read::ZlibEncoder, Compression};
use sha1::{Digest, Sha1};

const OBJECTS_PATH: &str = ".git/objects";

#[derive(PartialEq)]
pub enum ObjectType {
    Blob,
    Tree,
    //Commit,
}

impl Display for ObjectType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ObjectType::Blob => write!(f, "blob"),
            ObjectType::Tree => write!(f, "tree"),
            // ObjectType::Commit => write!(f, "commit"),
        }
    }
}

#[derive(Clone)]
pub struct Header {
    pub typ: String,
    pub size: usize,
}

pub struct ObjectFile {
    header: Option<Header>,
    content: Vec<u8>,
    content_read: bool,
    path: PathBuf,
    decoder: BufReader<ZlibDecoder<fs::File>>,
}

impl ObjectFile {
    pub fn read(hash: &str) -> anyhow::Result<Self> {
        let path = Self::hash_to_path(hash);
        let f =
            fs::File::open(&path).with_context(|| format!("opening file {}", path.display()))?;

        let decoder = BufReader::new(ZlibDecoder::new(f));

        Ok(Self {
            header: None,
            content: Vec::new(),
            content_read: false,
            path,
            decoder,
        })
    }

    /// Returns `Header` containing type of the decompressed object file and size of it content
    pub fn get_header(&mut self) -> anyhow::Result<Header> {
        if self.header.is_none() {
            self.read_header()?;
        }
        Ok(self.header.clone().expect("header is set here"))
    }

    /// Returns content of the decompressed object file
    pub fn get_content(&mut self) -> anyhow::Result<&[u8]> {
        if !self.content_read {
            self.read_content()?;
        }
        Ok(&self.content)
    }

    fn read_header(&mut self) -> anyhow::Result<()> {
        self.decoder
            .read_until(0, &mut self.content)
            .with_context(|| format!("reading object header in file {}", self.path.display()))?;

        let header = std::ffi::CStr::from_bytes_with_nul(&self.content)
            .expect("should be null terminated string")
            .to_str()
            .context("file header is not valid UTF-8")?;

        let Some((typ, size)) = header.split_once(' ') else {
            anyhow::bail!("incorrect object header: {}", header)
        };

        let size = size
            .parse::<usize>()
            .context("parsing object size in header")?;

        self.header = Some(Header {
            typ: typ.to_owned(),
            size,
        });

        Ok(())
    }

    fn read_content(&mut self) -> anyhow::Result<()> {
        if self.header.is_none() {
            self.read_header()?;
        }

        let size = self.header.as_ref().expect("header is set here").size;

        self.content.clear();
        self.content.reserve_exact(size);
        self.content.resize(size, 0);

        self.decoder
            .read_exact(&mut self.content)
            .context("reading object data")?;

        let n = self
            .decoder
            .read(&mut [0])
            .context("ensuring that object was completely read")?;

        anyhow::ensure!(
            n == 0,
            "object size is {n} bytes larger than stated in object header"
        );

        self.content_read = true;

        Ok(())
    }

    /// Computes and returns object hash ID
    pub fn hash(mut r: impl Read) -> anyhow::Result<[u8; 20]> {
        let mut buf = Vec::new();
        r.read_to_end(&mut buf)
            .context("reading data to compute SHA1 digest")?;
        let digest = Sha1::digest(&buf);

        Ok(digest.into())
    }

    /// Compresses and write object's data to disk. Returns object hash ID.
    pub fn write<R>(r: R) -> anyhow::Result<[u8; 20]>
    where
        R: Read + std::clone::Clone,
    {
        let digest = ObjectFile::hash(r.clone())?;

        let mut encoder = ZlibEncoder::new(r, Compression::fast());

        let mut compressed = Vec::new();

        encoder
            .read_to_end(&mut compressed)
            .context("compressing the file")?;

        let hash = hex::encode(digest);
        let dir = &hash[..2]; // first 2 chars of the digest
        let filename = &hash[2..]; // rest of the digest

        let mut path = PathBuf::from(OBJECTS_PATH);
        path.push(dir);

        fs::create_dir_all(&path)
            .with_context(|| format!("creating directory {}", path.display()))?;

        path.push(filename);

        let mut f =
            fs::File::create(&path).with_context(|| format!("creating file {}", path.display()))?;
        f.write_all(&compressed)?;

        Ok(digest)
    }

    fn hash_to_path(hash: &str) -> PathBuf {
        let dir = &hash[..2];
        let file = &hash[2..];
        let mut path = PathBuf::from(OBJECTS_PATH);
        path.push(dir);
        path.push(file);
        path
    }
}
