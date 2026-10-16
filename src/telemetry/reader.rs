//! Bounded reverse reads of the fixed application log set. No client value becomes a path.
use std::{
    fs::{self, File, Metadata},
    io::{self, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::Instant,
};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{FILE_NAME, MAX_RECORD_BYTES};

const SCAN_BYTES: usize = 1024 * 1024;
const RESPONSE_BYTES: usize = 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    #[error("invalid log query")]
    InvalidQuery,
    #[error("log cursor expired")]
    CursorExpired,
    #[error("log query deadline exceeded")]
    Timeout,
    #[error("log file unavailable")]
    Io(#[source] io::Error),
}

#[derive(Clone)]
pub struct LogReader {
    directory: PathBuf,
    max_files: usize,
    run_id: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Cursor {
    version: u8,
    run_id: String,
    file_id: String,
    offset: u64,
    size: u64,
    min_level: Option<String>,
    request_id: Option<String>,
}

#[derive(Default)]
struct Query {
    file_id: Option<String>,
    cursor: Option<String>,
    limit: usize,
    min_level: Option<String>,
    request_id: Option<String>,
}

struct Identity {
    id: String,
    timestamp: String,
}

impl LogReader {
    pub fn new(directory: PathBuf, max_files: usize, run_id: String) -> Self {
        Self {
            directory,
            max_files,
            run_id,
        }
    }

    pub fn files(&self, deadline: Instant) -> Result<Value, ReadError> {
        regular_directory(&self.directory).map_err(ReadError::Io)?;
        let mut data = Vec::new();
        for index in 0..=self.max_files {
            check_deadline(deadline)?;
            let Some((mut file, size)) = open_file(&file_path(&self.directory, index))? else {
                continue;
            };
            if size == 0 {
                continue;
            }
            match identity(&mut file)? {
                Some(identity) => data.push(json!({"fileId":identity.id,"active":index == 0,"sizeBytes":size.to_string(),"firstTimestamp":identity.timestamp})),
                None => data.push(json!({"fileId":null,"active":index == 0,"sizeBytes":size.to_string(),"firstTimestamp":null,"state":"unreadable"})),
            }
        }
        check_deadline(deadline)?;
        Ok(json!({"data":data,"nextCursor":null}))
    }

    pub fn logs(
        &self,
        parameters: &[(String, String)],
        deadline: Instant,
    ) -> Result<Value, ReadError> {
        check_deadline(deadline)?;
        let query = Query::parse(parameters)?;
        let cursor = self.cursor(&query)?;
        regular_directory(&self.directory).map_err(ReadError::Io)?;
        let requested = cursor
            .as_ref()
            .map(|value| value.file_id.as_str())
            .or(query.file_id.as_deref());
        let Some((mut file, id, size)) = self.locate(requested, deadline)? else {
            if requested.is_some() {
                return Err(ReadError::CursorExpired);
            }
            return Ok(
                json!({"data":[],"nextCursor":null,"fileId":null,"skippedRecords":0,"scannedBytes":"0"}),
            );
        };
        let position = cursor.as_ref().map_or(size, |cursor| cursor.offset);
        if cursor
            .as_ref()
            .is_some_and(|cursor| size < cursor.size || position > cursor.size)
        {
            return Err(ReadError::CursorExpired);
        }
        let minimum = cursor
            .as_ref()
            .and_then(|cursor| cursor.min_level.as_deref())
            .or(query.min_level.as_deref());
        let request_id = cursor
            .as_ref()
            .and_then(|cursor| cursor.request_id.as_deref())
            .or(query.request_id.as_deref());
        let start = position.saturating_sub(SCAN_BYTES as u64);
        let mut bytes =
            vec![0; usize::try_from(position - start).map_err(|_error| ReadError::InvalidQuery)?];
        file.seek(SeekFrom::Start(start)).map_err(ReadError::Io)?;
        file.read_exact(&mut bytes)
            .map_err(|_error| ReadError::CursorExpired)?;
        check_deadline(deadline)?;
        let page = scan(&bytes, start, query.limit, minimum, request_id, deadline)?;
        let current_size = file.metadata().map_err(ReadError::Io)?.len();
        if current_size < size
            || identity(&mut file)?
                .as_ref()
                .is_none_or(|current| current.id != id)
        {
            return Err(ReadError::CursorExpired);
        }
        // An open handle can outlive backup eviction on Unix; do not serve a file
        // which is no longer a member of the managed set after this short read.
        if self.locate(Some(&id), deadline)?.is_none() {
            return Err(ReadError::CursorExpired);
        }
        check_deadline(deadline)?;
        let next =
            if page.offset > 0 {
                let cursor = Cursor {
                    version: 1,
                    run_id: self.run_id.clone(),
                    file_id: id.clone(),
                    offset: page.offset,
                    size: cursor.as_ref().map_or(size, |cursor| cursor.size),
                    min_level: minimum.map(str::to_owned),
                    request_id: request_id.map(str::to_owned),
                };
                Some(URL_SAFE_NO_PAD.encode(
                    serde_json::to_vec(&cursor).expect("cursor contains serializable scalars"),
                ))
            } else {
                None
            };
        Ok(
            json!({"data":page.data,"nextCursor":next,"fileId":id,"skippedRecords":page.skipped,"scannedBytes":bytes.len().to_string()}),
        )
    }

    fn cursor(&self, query: &Query) -> Result<Option<Cursor>, ReadError> {
        let Some(encoded) = &query.cursor else {
            return Ok(None);
        };
        if encoded.len() > 2048 {
            return Err(ReadError::InvalidQuery);
        }
        let decoded = URL_SAFE_NO_PAD
            .decode(encoded)
            .map_err(|_error| ReadError::InvalidQuery)?;
        let cursor: Cursor =
            serde_json::from_slice(&decoded).map_err(|_error| ReadError::InvalidQuery)?;
        if cursor.version != 1
            || !valid_id(&cursor.file_id)
            || cursor.offset > cursor.size
            || cursor
                .min_level
                .as_deref()
                .is_some_and(|value| level(value).is_none())
            || cursor
                .request_id
                .as_deref()
                .is_some_and(|value| !valid_id(value))
        {
            return Err(ReadError::InvalidQuery);
        }
        if cursor.run_id != self.run_id {
            return Err(ReadError::CursorExpired);
        }
        if query
            .file_id
            .as_ref()
            .is_some_and(|id| id != &cursor.file_id)
            || query
                .min_level
                .as_ref()
                .is_some_and(|value| Some(value) != cursor.min_level.as_ref())
            || query
                .request_id
                .as_ref()
                .is_some_and(|value| Some(value) != cursor.request_id.as_ref())
        {
            return Err(ReadError::InvalidQuery);
        }
        Ok(Some(cursor))
    }

    fn locate(
        &self,
        requested: Option<&str>,
        deadline: Instant,
    ) -> Result<Option<(File, String, u64)>, ReadError> {
        let last = if requested.is_none() {
            0
        } else {
            self.max_files
        };
        for index in 0..=last {
            check_deadline(deadline)?;
            let Some((mut file, size)) = open_file(&file_path(&self.directory, index))? else {
                continue;
            };
            if size == 0 {
                continue;
            }
            let id = identity(&mut file)?;
            if let Some(id) = id {
                if requested.is_none_or(|requested| requested == id.id) {
                    return Ok(Some((file, id.id, size)));
                }
            } else if requested.is_none() {
                return Err(ReadError::CursorExpired);
            }
        }
        Ok(None)
    }
}

impl Query {
    fn parse(parameters: &[(String, String)]) -> Result<Self, ReadError> {
        let mut query = Self {
            limit: 100,
            ..Self::default()
        };
        let mut seen = std::collections::HashSet::new();
        for (name, value) in parameters {
            if !seen.insert(name) {
                return Err(ReadError::InvalidQuery);
            }
            match name.as_str() {
                "fileId" if valid_id(value) => query.file_id = Some(value.clone()),
                "cursor" if !value.is_empty() && value.len() <= 2048 => {
                    query.cursor = Some(value.clone());
                },
                "limit" if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) => {
                    query.limit = value
                        .parse::<usize>()
                        .ok()
                        .filter(|limit| (1..=200).contains(limit))
                        .ok_or(ReadError::InvalidQuery)?;
                },
                "minLevel" if level(value).is_some() => query.min_level = Some(value.clone()),
                "requestId" if valid_id(value) => query.request_id = Some(value.clone()),
                _ => return Err(ReadError::InvalidQuery),
            }
        }
        Ok(query)
    }
}

struct Page {
    data: Vec<Value>,
    offset: u64,
    skipped: usize,
}

fn scan(
    bytes: &[u8],
    start: u64,
    limit: usize,
    minimum: Option<&str>,
    request_id: Option<&str>,
    deadline: Instant,
) -> Result<Page, ReadError> {
    let first = if start == 0 {
        0
    } else {
        bytes
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(bytes.len(), |position| position + 1)
    };
    let mut page = Page {
        data: Vec::new(),
        offset: start + first as u64,
        skipped: 0,
    };
    let mut end = bytes.len();
    let mut response_bytes = 4096; // Reserve the bounded cursor and envelope, not only array contents.
    while end > first {
        check_deadline(deadline)?;
        let complete = bytes[end - 1] == b'\n';
        let body_end = end - usize::from(complete);
        let line_start = bytes[first..body_end]
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(first, |position| first + position + 1);
        let line = &bytes[line_start..end];
        let record = complete.then(|| parse_record(line)).flatten();
        if let Some(record) = record {
            let matches = minimum.is_none_or(|minimum| {
                record["level"]
                    .as_str()
                    .and_then(level)
                    .is_some_and(|actual| actual >= level(minimum).unwrap_or(0))
            }) && request_id
                .is_none_or(|id| record["requestId"].as_str() == Some(id));
            if matches {
                let encoded = serde_json::to_vec(&record).expect("parsed JSON serializes");
                if page.data.len() >= limit || response_bytes + encoded.len() + 1 > RESPONSE_BYTES {
                    page.offset = start + end as u64;
                    return Ok(page);
                }
                response_bytes += encoded.len() + 1;
                page.data.push(record);
            }
        } else {
            page.skipped += 1;
        }
        end = line_start;
    }
    // An overlong malformed line may fill a scan window without any newline.
    if first == bytes.len() && !bytes.is_empty() {
        page.offset = start;
        page.skipped += 1;
    }
    Ok(page)
}

fn parse_record(line: &[u8]) -> Option<Value> {
    if line.len() > MAX_RECORD_BYTES || line.last() != Some(&b'\n') {
        return None;
    }
    let mut record: Value = serde_json::from_slice(line).ok()?;
    if record["schemaVersion"] != 1
        || !valid_id(record["logId"].as_str()?)
        || level(record["level"].as_str()?).is_none()
        || chrono::DateTime::parse_from_rfc3339(record["timestamp"].as_str()?).is_err()
        || !super::encoding::approved_event(record["event"].as_str()?)
    {
        return None;
    }
    record
        .as_object_mut()?
        .retain(|key, _| super::encoding::approved_record_field(key));
    Some(record)
}

fn identity(file: &mut File) -> Result<Option<Identity>, ReadError> {
    file.seek(SeekFrom::Start(0)).map_err(ReadError::Io)?;
    let mut bytes = Vec::with_capacity(MAX_RECORD_BYTES);
    file.take(MAX_RECORD_BYTES as u64)
        .read_to_end(&mut bytes)
        .map_err(ReadError::Io)?;
    let Some(end) = bytes.iter().position(|byte| *byte == b'\n') else {
        return Ok(None);
    };
    let Some(record) = parse_record(&bytes[..=end]) else {
        return Ok(None);
    };
    Ok(Some(Identity {
        id: record["logId"].as_str().unwrap_or_default().to_owned(),
        timestamp: record["timestamp"].as_str().unwrap_or_default().to_owned(),
    }))
}

fn open_file(path: &Path) -> Result<Option<(File, u64)>, ReadError> {
    let before = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(ReadError::Io(error)),
    };
    if !is_regular(&before) {
        return Err(ReadError::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            "log file is not regular",
        )));
    }
    let file = File::open(path).map_err(ReadError::Io)?;
    let opened = file.metadata().map_err(ReadError::Io)?;
    let after = fs::symlink_metadata(path).map_err(ReadError::Io)?;
    if !is_regular(&opened)
        || !is_regular(&after)
        || !same_file(&before, &opened)
        || !same_file(&opened, &after)
    {
        return Err(ReadError::CursorExpired);
    }
    Ok(Some((file, opened.len())))
}

#[cfg(unix)]
fn same_file(a: &Metadata, b: &Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    a.dev() == b.dev() && a.ino() == b.ino()
}

#[cfg(not(unix))]
fn same_file(a: &Metadata, b: &Metadata) -> bool {
    match (a.created(), b.created()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

pub(super) fn regular_directory(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_dir() && !is_link(&metadata) {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "log directory is not regular",
        ))
    }
}

pub(super) fn is_regular(metadata: &Metadata) -> bool { metadata.is_file() && !is_link(metadata) }

#[cfg(windows)]
fn is_link(metadata: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & 0x400 != 0
}

#[cfg(not(windows))]
fn is_link(metadata: &Metadata) -> bool { metadata.file_type().is_symlink() }

pub(super) fn file_path(directory: &Path, index: usize) -> PathBuf {
    directory.join(if index == 0 {
        FILE_NAME.to_owned()
    } else {
        format!("{FILE_NAME}.{index}")
    })
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

fn level(value: &str) -> Option<u8> {
    match value {
        "trace" => Some(0),
        "debug" => Some(1),
        "info" => Some(2),
        "warn" => Some(3),
        "error" => Some(4),
        _ => None,
    }
}

fn check_deadline(deadline: Instant) -> Result<(), ReadError> {
    if Instant::now() >= deadline {
        Err(ReadError::Timeout)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
