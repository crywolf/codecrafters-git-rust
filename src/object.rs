use std::fs::File;
use std::io::{prelude::*, BufReader};

use anyhow::{bail, Context};
use flate2::read::ZlibDecoder;

const OBJECTS_PATH: &str = ".git/objects"; // TODO remove duplication

pub struct ObjectFile {
    pub typ: Option<String>,
    pub size: Option<usize>,
    path: String,
    buf: Vec<u8>,
    decoder: BufReader<ZlibDecoder<File>>,
}

impl ObjectFile {
    pub fn new(hash: &str) -> anyhow::Result<Self> {
        let path = Self::hash_to_path(hash);
        let f = File::open(&path).with_context(|| format!("opening file {path}"))?;

        let z = ZlibDecoder::new(f);
        let decoder = BufReader::new(z);

        Ok(Self {
            typ: None,
            size: None,
            path,
            buf: Vec::new(),
            decoder,
        })
    }

    pub fn read_header(&mut self) -> anyhow::Result<()> {
        self.decoder
            .read_until(0, &mut self.buf)
            .with_context(|| format!("reading object header in file {}", self.path))?;

        let header = std::ffi::CStr::from_bytes_with_nul(&self.buf)
            .expect("should be null terminated string")
            .to_str()
            .context("file header is not valid UTF-8")?;

        let (typ, size) = header
            .split_once(' ')
            .with_context(|| format!("parsing object header {header}"))?;

        let size = size
            .parse::<usize>()
            .context("parsing object size in header")?;

        self.typ = Some(typ.to_owned());
        self.size = Some(size);

        Ok(())
    }

    pub fn read_content(&mut self) -> anyhow::Result<()> {
        if self.size.is_none() {
            bail!("file header must be read before reading content")
        }
        let size = self.size.expect("size was already parsed");

        self.buf.clear();
        self.buf.reserve_exact(size);
        self.buf.resize(size, 0);

        self.decoder
            .read_exact(&mut self.buf)
            .context("reading object data")?;

        let n = self
            .decoder
            .read(&mut [0])
            .context("ensuring that object was completely read")?;

        anyhow::ensure!(
            n == 0,
            "object size is {n} bytes larger than stated in object header"
        );

        Ok(())
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.buf
    }

    fn hash_to_path(hash: &str) -> String {
        let dir = &hash[..2];
        let file = &hash[2..];
        let path = format!("{OBJECTS_PATH}/{dir}/{file}");
        path
    }
}
