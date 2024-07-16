use anyhow::{Context, Ok};
use flate2::read::ZlibDecoder;
use reqwest::StatusCode;
use std::fs;
use std::io::{BufRead, Read};
use std::path::{Path, PathBuf};
use std::{fmt::Write, io::BufReader};

use bytes::{Buf, BufMut, Bytes, BytesMut};

use crate::{
    commands,
    object::{self, ObjectFile, ObjectType},
};

const SERVICE_NAME: &str = "git-upload-pack";

pub fn invoke(repository_url: &str, dir: Option<PathBuf>) -> anyhow::Result<()> {
    // References:
    // https://www.git-scm.com/docs/http-protocol
    // https://www.git-scm.com/book/en/v2/Git-Internals-Transfer-Protocols
    // https://github.com/git/git/blob/795ea8776befc95ea2becd8020c7a284677b4161/Documentation/gitformat-pack.txt
    // https://github.com/git/git/blob/795ea8776befc95ea2becd8020c7a284677b4161/Documentation/gitprotocol-pack.txt
    // https://github.com/git/git/blob/795ea8776befc95ea2becd8020c7a284677b4161/Documentation/gitprotocol-common.txt
    // https://codewords.recurse.com/issues/three/unpacking-git-packs
    // https://scribe.rip/@concertdaw/sneaky-git-number-encoding-ddcc5db5329f

    let mut repository_url = repository_url.to_string();

    // strip trailing /
    if repository_url.ends_with('/') {
        repository_url.pop();
    }

    let dir = match dir {
        Some(dir) => dir,
        None => {
            // determine default dir name from repository url
            let mut repo_name = repository_url
                .rsplit('/')
                .next()
                .ok_or(anyhow::anyhow!("could not determine output directory"))?
                .to_string();
            if repo_name.ends_with(".git") {
                repo_name.truncate(repo_name.len() - 4)
            }
            PathBuf::from(repo_name)
        }
    };

    commands::init::create_git_dirs(Some(dir.as_path())).with_context(|| {
        format!(
            "initializing Git repository in '{}' directory",
            dir.display()
        )
    })?;

    let (mut pack_data, head_ref_hash) =
        get_pack_data(repository_url).context("getting pack from remote")?;

    println!("Cloning into '{}'...", dir.display());

    let num_obj = pack_data.get_u32();
    println!("Pack contains {num_obj} objects");

    let mut received_objects: usize = 0;
    let mut resolved_deltas: usize = 0;

    for _ in 0..num_obj {
        /*
         Valid object types are:
          - OBJ_COMMIT (1)
          - OBJ_TREE (2)
          - OBJ_BLOB (3)
          - OBJ_TAG (4)
          - OBJ_OFS_DELTA (6)
          - OBJ_REF_DELTA (7)
        */
        let b = pack_data.get_u8();
        let mut msb = b & 0b1000_0000 > 0;
        let obj_type = match (b & 0b0111_0000) >> 4 {
            1 => ObjectType::Commit,
            2 => ObjectType::Tree,
            3 => ObjectType::Blob,
            4 => ObjectType::Tag,
            6 => ObjectType::OfsDelta,
            7 => ObjectType::RefDelta,
            other => anyhow::bail!("Unknown or unsupported object: {other}"),
        };
        let mut obj_size = (b & 0b0000_1111) as usize;
        let mut shift = 4;
        while msb {
            let b = pack_data.get_u8();
            if b & 0b1000_0000 == 0 {
                msb = false;
            }
            obj_size += ((b & 0b0111_1111) as usize) << shift;
            shift += 7;
        }

        let mut base_obj_hash = String::new();
        if obj_type == ObjectType::RefDelta {
            // 20-byte name of the base object
            base_obj_hash = hex::encode(pack_data.get(..20).ok_or(anyhow::anyhow!(
                "could not get OBJ_REF_DELTA base object name"
            ))?);
            pack_data.advance(20);
        }

        let mut obj_reader = pack_data.as_ref().reader();
        let decoder = ZlibDecoder::new(&mut obj_reader);
        let mut obj = ObjectFile {
            header: object::Header {
                typ: obj_type,
                size: obj_size,
            },
            reader: decoder,
        };

        if obj.header.typ == ObjectType::OfsDelta {
            // we skip OBJ_OFS_DELTA objects
            // just read out the compressed delta data from the reader
            std::io::copy(&mut obj.reader, &mut std::io::sink())
                .context("streaming object's data to sink")?;
            pack_data.advance(obj.reader.total_in() as usize);
            println!("OBJ_OFS_DELTA objects are not supported");
        } else if obj.header.typ == ObjectType::RefDelta {
            // OBJ_REF_DELTA processing

            let mut base_obj = ObjectFile::read(&base_obj_hash, Some(dir.as_path()))?;

            process_delta_object(&dir, &mut obj, &mut base_obj)
                .context("processing delta object")?;
            pack_data.advance(obj.reader.total_in() as usize);
            resolved_deltas += 1;
        } else {
            // Regular object (blob, tree, commmit)

            obj.write(Some(dir.as_path()))?;
            pack_data.advance(obj.reader.total_in() as usize);
            received_objects += 1;
        }
    }

    anyhow::ensure!(pack_data.remaining() == 20, "cannot get pack checksum");
    println!(
        "Pack checksum: {}",
        hex::encode((pack_data.get(..)).context("reading checksum")?)
    );

    // reconstruct files according to the HEAD
    let head_commit_obj = ObjectFile::read(&head_ref_hash, Some(dir.as_path()))?;
    anyhow::ensure!(
        head_commit_obj.header.typ == ObjectType::Commit,
        "HEAD does not point to commit"
    );

    let mut head_tree_hash = String::new();

    let mut head_commit = BufReader::new(head_commit_obj.reader);
    let mut line = String::new();
    while head_commit
        .read_line(&mut line)
        .context("reading commit line")?
        > 0
    {
        if let Some((k, v)) = line.split_once(' ') {
            if *k.to_string() == ObjectType::Tree.to_string() {
                head_tree_hash = v.trim().to_string();
                break;
            }
        } else {
            anyhow::bail!("failed to parse commit line");
        };
    }

    if head_tree_hash.is_empty() {
        anyhow::bail!("could not get tree from commit {}", head_ref_hash);
    }

    reconstruct_repo_files(dir.as_path(), dir.as_path(), &head_tree_hash)
        .context("reconstructing files")?;

    println!("Received objects: {}", received_objects);
    println!("Resolved deltas: {}", resolved_deltas);

    Ok(())
}

fn get_pack_data(repository_url: String) -> anyhow::Result<(Bytes, String)> {
    // GET $GIT_URL/info/refs?service=git-upload-pack HTTP/1.0
    let url = format!("{repository_url}/info/refs?service={SERVICE_NAME}");

    let client = reqwest::blocking::Client::new();

    let resp = client.get(&url).send()?;

    // Clients MUST validate the status code is either 200 OK or 304 Not Modified.
    if !resp.status().is_success()
        || (resp.status() != StatusCode::OK && resp.status() != StatusCode::NOT_MODIFIED)
    {
        anyhow::bail!(
            "calling remote repository server {url} failed: {}",
            resp.status()
        )
    }

    // The Content-Type MUST be application/x-$servicename-advertisement.
    // Clients SHOULD fall back to the dumb protocol if another content type is returned.
    // Clients MUST NOT continue if they do not support the dumb protocol.
    let headers = resp.headers();
    if let Some(content_type) = headers.get(reqwest::header::CONTENT_TYPE) {
        if content_type != format!("application/x-{SERVICE_NAME}-advertisement").as_str() {
            anyhow::bail!(
                "incorrect Content-Type header {}",
                content_type
                    .to_str()
                    .context("checking Content-Type header")?
            )
        }
    } else {
        anyhow::bail!("missing Content-Type header while calling {url}")
    }

    let mut data = resp
        .bytes()
        .with_context(|| format!("reading response body bytes {url}"))?;

    /*
    // Response data example:
    001e# service=git-upload-pack\n
    0000
    01556c073b08f7987018cbb2cb9a5747c84913b3608e HEAD\0multi_ack thin-pack side-band side-band-64k ofs-delta shallow deepen-since deepen-not deepen-relative no-progress include-tag multi_ack_detailed allow-tip-sha1-in-want allow-reachable-sha1-in-want no-done symref=HEAD:refs/heads/master filter object-format=sha1 agent=git/github-e62f56720ee6\n
    003f6c073b08f7987018cbb2cb9a5747c84913b3608e refs/heads/master\n
    003ded6c73fc16578ec53ea374585df2b965ce9f4a31 refs/tags/1.0.0\n
    0000
    */

    // Clients MUST validate the first five bytes of the response entity matches the regex ^[0-9a-f]{4}#. If this test fails, clients MUST NOT continue.
    // Clients MUST verify the first pkt-line is # service=$servicename. Servers MUST set $servicename to be the request parameter value.
    // Servers SHOULD include an LF at the end of this line. Clients MUST ignore an LF at the end of the line.
    // Servers MUST terminate the response with the magic 0000 end pkt-line marker.
    let first_line = b"001e# service=git-upload-pack\n0000";
    anyhow::ensure!(
        &data.starts_with(first_line),
        "invalid first pkt-line in response"
    );
    data.advance(first_line.len());

    // The returned response is a pkt-line stream describing each ref and its known value.
    // The stream SHOULD be sorted by name according to the C locale ordering.
    // The stream SHOULD include the default ref named HEAD as the first ref.
    // The stream MUST include capability declarations behind a NUL on the first ref.
    let _line_len = data.get_u32();

    let head_ref_hash = std::str::from_utf8(
        data.get(0..40)
            .context("reading 40 bytes of HEAD ref hash")?,
    )
    .context("reading HEAD ref hash")?;

    let ref_name = data.get(40..46).context("chcecking presence of HEAD ref")?;
    anyhow::ensure!(
        ref_name.starts_with(b" HEAD\0"),
        "HEAD ref is not present in response"
    );

    // POST $GIT_URL/git-upload-pack HTTP/1.0
    let url = format!("{repository_url}/{SERVICE_NAME}");

    // The returned stream is the side-band-64k protocol supported by the git-upload-pack service, and the pack is embedded into stream 1.
    // Progress messages from the server side MAY appear in stream 2.
    let mut want = format!("0032want {head_ref_hash}\n");
    write!(want, "0000")?;
    writeln!(want, "0009done")?;

    let resp = client
        .post(&url)
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-git-upload-pack-request",
        )
        .body(want)
        .send()
        .context("requesting pack")?;

    if !resp.status().is_success() || resp.status() != StatusCode::OK {
        anyhow::bail!(
            "calling remote repository server {url} failed: {}",
            resp.status()
        )
    }

    let headers = resp.headers();
    if let Some(content_type) = headers.get(reqwest::header::CONTENT_TYPE) {
        if content_type != "application/x-git-upload-pack-result" {
            anyhow::bail!(
                "incorrect Content-Type header {}",
                content_type
                    .to_str()
                    .context("checking Content-Type header")?
            )
        }
    } else {
        anyhow::bail!("missing Content-Type header while calling {url}")
    }

    let mut data = resp
        .bytes()
        .with_context(|| format!("reading response body bytes {url}"))?;

    anyhow::ensure!(
        data.get(0..8).unwrap_or_default().starts_with(b"0008NAK\n"),
        "malformed pack header: missing NAK line"
    );
    anyhow::ensure!(
        data.get(8..12).unwrap_or_default().starts_with(b"PACK"),
        "malformed pack header: missing PACK"
    );
    data.advance(12);

    let version = data.get_u32();
    anyhow::ensure!(
        version == 2,
        "server returned unsupported pack version {version}"
    );

    Ok((data, head_ref_hash.to_string()))
}

// OBJ_REF_DELTA processing
fn process_delta_object(
    dir: impl AsRef<Path>,
    obj: &mut ObjectFile<ZlibDecoder<impl Read>>,
    base_obj: &mut ObjectFile<impl Read>,
) -> anyhow::Result<()> {
    let mut buf = Vec::new();
    obj.reader
        .read_to_end(&mut buf)
        .context("reading object data to buffer")?;

    // delta_obj_data contains decompressed delta object data
    let mut delta_obj_data = BytesMut::new();
    delta_obj_data.extend_from_slice(&buf);

    /* The delta begins with the source and target lengths, both encoded as variable-length integers, which is useful for error checking,
    but is not essential.
    After this, there are a series of instructions, which may be either “copy” (MSB = 1) or “insert” (MSB = 0). */

    // get source lenth
    let b = delta_obj_data.get_u8();
    let mut msb = b & 0b1000_0000 > 0;
    let mut source_length = (b & 0b0111_1111) as usize;
    let mut shift = 7;
    while msb {
        let b = delta_obj_data.get_u8();
        if b & 0b1000_0000 == 0 {
            msb = false;
        }
        source_length += ((b & 0b0111_1111) as usize) << shift;
        shift += 7;
    }

    // get target length
    let b = delta_obj_data.get_u8();
    let mut msb = b & 0b1000_0000 > 0;
    let mut target_length = (b & 0b0111_1111) as usize;
    let mut shift = 7;
    while msb {
        let b = delta_obj_data.get_u8();
        if b & 0b1000_0000 == 0 {
            msb = false;
        }
        target_length += ((b & 0b0111_1111) as usize) << shift;
        shift += 7;
    }

    // base_obj_data containds decompressed base object data
    let mut base_obj_data = Vec::new();
    base_obj
        .reader
        .read_to_end(&mut base_obj_data)
        .context("reading base object data to buffer")?;

    anyhow::ensure!(
        base_obj.header.size == source_length,
        "incorrect base object length, expected {}, got {}",
        source_length,
        base_obj.header.size
    );

    // new_data contains data from base object with applied delta chunks
    let mut new_data = BytesMut::new();

    // read delta instructions
    loop {
        // get insert/copy instruction; msb 0 = insert, 1 = copy
        let instruction = delta_obj_data.get_u8();
        let msb = instruction >> 7; // MSB

        if msb == 0 {
            // INSERT
            // The insert instruction itself is the number of bytes to copy from the delta object to the output.
            // Since insert instructions all have their MSB set to 0, the maximum number of bytes to insert is 127.
            // So, if the instruction is 01001011, that means that we should read the next 75 bytes of the delta object and copy them to the output.

            let length = instruction as usize;
            let delta = delta_obj_data.get(0..length).ok_or(anyhow::anyhow!(
                "could not read delta object data to insert them"
            ))?;

            new_data.put(delta);
            delta_obj_data.advance(length);
        } else if msb == 1 {
            // COPY
            // Copy instructions signal that we should copy a consecutive chunk of bytes from the base object to the output.
            // There are two numbers that are necessary to perform this operation: the location (offset) of the first byte to copy, and the number of bytes to copy.
            // These are stored as little-endian variable-length integers after each copy instruction; however, their contents are compressed.
            //
            // Even though the byte offset is a 32-bit integer, Git only includes the non-zero bytes to save space,
            // and the last four bits of the copy instruction signal how many bytes to read.
            //
            // For example, let’s say that the last four bits of the copy instruction are 1010 and the next two bytes are 11010111 01001011.
            // This means that the byte offset is 01001011 00000000 11010111 00000000, which is 1,258,346,240.
            //
            // The copy length is interpreted the same way, with the middle three bits of the instruction signifying whether to advance the cursor or not,
            // just as the last four bits signify whether to advance the cursor when constructing the byte offset.

            let mut offset = 0;
            let mut length = 0;

            let flag = 0b0000_1111 & instruction; // ex. 1010
            for i in 0..4 {
                let mut b = 0;
                if flag & (1u8 << i) > 0 {
                    b = delta_obj_data.get_u8();
                }
                offset += (b as usize) << (i * 8);
            }

            let flag = 0b0111_0000 & instruction; // ex. 010
            for i in 0..3 {
                let mut b = 0;
                if flag & (1u8 << (i + 4)) > 0 {
                    b = delta_obj_data.get_u8();
                }
                length += (b as usize) << (i * 8);
            }

            let delta = base_obj_data
                .get(offset..offset + length)
                .ok_or(anyhow::anyhow!(
                    "could not read base object data to copy them"
                ))?;
            new_data.put(delta);
        } else {
            anyhow::bail!("incorrect delta instruction {instruction}");
        }

        if delta_obj_data.remaining() == 0 {
            break;
        }
    }

    anyhow::ensure!(
        new_data.len() == target_length,
        "incorrect new base object length, expected {}, got {}",
        target_length,
        new_data.len()
    );

    let mut new_obj = ObjectFile {
        header: object::Header {
            typ: base_obj.header.typ.clone(),
            size: new_data.len(),
        },
        reader: new_data.reader(),
    };

    new_obj.write(Some(dir.as_ref()))?;

    Ok(())
}

fn reconstruct_repo_files(
    clone_dir: impl AsRef<Path>,
    current_dir: impl AsRef<Path>,
    tree_hash: &str,
) -> anyhow::Result<()> {
    let clone_dir = clone_dir.as_ref();
    let current_dir = current_dir.as_ref();

    let mut tree_obj = ObjectFile::read(tree_hash, Some(clone_dir))
        .with_context(|| format!("opening tree file {tree_hash}"))?;

    let typ = tree_obj.header.typ;
    anyhow::ensure!(
        typ == ObjectType::Tree,
        "incorrect tree object type '{typ}'"
    );

    loop {
        // parsing tree object copied from list_tree function in ls-tree command
        let mut buf = Vec::new();
        let n = tree_obj
            .reader
            .read_until(0, &mut buf)
            .context("reading mode and name for tree item")?;
        if n == 0 {
            break;
        }

        let item = std::ffi::CStr::from_bytes_with_nul(&buf)
            .expect("should be null terminated string")
            .to_str()
            .context("mode and name in tree item is not valid UTF-8")?;

        let (mode, name) = item
            .split_once(' ')
            .with_context(|| format!("parsing object mode and name from {item}"))?;

        let mut hash = [0; 20];
        tree_obj
            .reader
            .read_exact(&mut hash)
            .context("reading sha hash of tree item")?;
        let hash = hex::encode(hash);

        let mut kind = ObjectType::Blob;
        if mode.starts_with('4') {
            kind = ObjectType::Tree;
        }

        let mut path = PathBuf::from(current_dir);
        path.push(name);

        if kind == ObjectType::Tree {
            fs::create_dir(&path).with_context(|| format!("creating dir {}", path.display()))?;
            reconstruct_repo_files(clone_dir, &path, &hash)
                .with_context(|| format!("witing content of dir {}", path.display()))?
        } else {
            let mut blob = ObjectFile::read(&hash, Some(clone_dir))?;
            let mut f = fs::File::create(&path)
                .with_context(|| format!("creating file {}", path.display()))?;
            std::io::copy(&mut blob.reader, &mut f)
                .with_context(|| format!("witing content to file {}", path.display()))?;
        }
    }

    Ok(())
}
