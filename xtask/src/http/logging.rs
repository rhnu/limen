//! Exercise log reads and rotation against supervised HTTP processes and real temporary files.
use std::{fs, io::Write};

use anyhow::{Context, Result, ensure};
use serde_json::json;

use super::fixture::{Laboratory, Options, request};

pub async fn run(laboratory: &mut Laboratory<'_>) -> Result<()> {
    laboratory
        .with_fixture(Options::default(), async |item| {
            let directory = item.application_log_dir();
            let backup = directory.join("limen.jsonl.1");
            let mut records = [
                record("test-log-1", "info"),
                record("test-log-2", "warn"),
                record("test-log-3", "error"),
            ]
            .concat();
            records.extend_from_slice(b"not-a-record\n{\"unfinished\":");
            fs::write(&backup, &records).context("write synthetic backup log")?;
            let page = item
                .manager("/admin/v1/logs?fileId=test-log-1&limit=1")
                .await?;
            ensure!(
                page.status == 200 && page.body["data"][0]["logId"] == "test-log-3",
                "reverse log page returned wrong newest record"
            );
            ensure!(
                page.body["skippedRecords"] == 2,
                "corrupt complete and partial records were not counted"
            );
            let cursor = page.body["nextCursor"]
                .as_str()
                .context("log cursor absent")?
                .to_owned();
            let renamed = directory.join("limen.jsonl.2");
            fs::rename(&backup, &renamed)
                .context("rename synthetic backup within managed file set")?;
            let second = item
                .manager(&format!("/admin/v1/logs?cursor={cursor}&limit=1"))
                .await?;
            ensure!(
                second.status == 200 && second.body["data"][0]["logId"] == "test-log-2",
                "file identity did not survive rollover rename"
            );
            item.manager(&format!("/admin/v1/logs?cursor={cursor}&minLevel=error"))
                .await?
                .error(400, "invalid_query", "cursor accepted different filter")?;
            fs::OpenOptions::new()
                .write(true)
                .open(&renamed)?
                .set_len(0)
                .context("truncate selected backup")?;
            item.manager(&format!("/admin/v1/logs?cursor={cursor}"))
                .await?
                .error(410, "cursor_expired", "truncated log cursor did not expire")?;
            fs::write(directory.join("limen.jsonl.3"), b"corrupt-first-line\n")
                .context("write malformed log identity")?;
            let files = item.manager("/admin/v1/logs/files").await?;
            ensure!(
                files.status == 200
                    && files.body["data"]
                        .as_array()
                        .context("log file list")?
                        .iter()
                        .any(|file| file["state"] == "unreadable" && file["fileId"].is_null()),
                "malformed first record was guessed as a file identity"
            );
            let large = directory.join("limen.jsonl.4");
            let mut file = fs::File::create(&large).context("create bounded-scan log fixture")?;
            file.write_all(&record("test-large-1", "info"))?;
            file.write_all(&vec![b'x'; 1024 * 1024 + 64])?;
            file.flush()?;
            drop(file);
            let empty = item
                .manager("/admin/v1/logs?fileId=test-large-1&minLevel=error")
                .await?;
            ensure!(
                empty.status == 200
                    && empty.body["data"].as_array().is_some_and(Vec::is_empty)
                    && empty.body["nextCursor"].is_string()
                    && empty.body["scannedBytes"] == "1048576",
                "scan budget must preserve a continuation even without matches"
            );
            let blocked = directory.join("limen.jsonl.5");
            fs::create_dir(&blocked).context("create non-regular managed entry")?;
            item.manager("/admin/v1/logs/files").await?.error(
                503,
                "storage_unavailable",
                "log reader accepted non-regular managed path",
            )?;
            fs::remove_dir(&blocked).context("remove synthetic directory")?;
            check_symlink(item, &directory).await?;
            Ok(())
        })
        .await?;
    rotation(laboratory).await
}

async fn rotation(laboratory: &mut Laboratory<'_>) -> Result<()> {
    let names: Vec<String> = (0..16)
        .map(|index| format!("diagnostic_{index}_{}", "x".repeat(40)))
        .collect();
    let borrowed: Vec<&str> = names.iter().map(String::as_str).collect();
    let script = format!(
        r"import {{endpoint}} from './fixture-data.js';
        import {{defineGateway}} from 'limen:gateway';
        const names = {};
        function observe(ctx) {{ for (const name of names) ctx.observe.record(name); }}
        export default defineGateway(gateway=>{{ const p=gateway.provider('p',{{endpoint}}); p.model('chat',{{upstream:'real-model'}}); gateway.onRequest(observe); gateway.onFinish(observe); }});",
        serde_json::to_string(&names)?
    );
    laboratory
        .with_fixture(
            Options {
                config: Some(&script),
                observation_names: &borrowed,
                log_max_file_bytes: Some(1024 * 1024),
                log_max_files: Some(1),
                ..Options::default()
            },
            async |item| {
                let directory = item.application_log_dir();
                let backup = directory.join("limen.jsonl.1");
                for _ in 0..300 {
                    let reply = item.request(request(&[])).await?;
                    ensure!(
                        reply.status == 200,
                        "generation failed while filling size-rotated logs"
                    );
                    item.record(reply.request_id()?).await?;
                    if backup.is_file() {
                        break;
                    }
                }
                ensure!(
                    backup.is_file(),
                    "bounded generation events did not trigger size rollover"
                );
                let metadata = fs::metadata(&backup).context("read rolled log size")?;
                ensure!(
                    metadata.len() >= 1024 * 1024 && metadata.len() <= 1024 * 1024 + 16 * 1024,
                    "size rollover exceeded one bounded record"
                );
                fs::remove_file(&backup).context("remove synthetic rolled backup")?;
                fs::create_dir(&backup).context("block rollover destination with directory")?;
                let mut failed = false;
                for _ in 0..300 {
                    let reply = item.request(request(&[])).await?;
                    ensure!(
                        reply.status == 200,
                        "log I/O failure incorrectly blocked generation"
                    );
                    item.record(reply.request_id()?).await?;
                    let status = item.manager("/admin/v1/status").await?;
                    if status.body["data"]["logging"]["file"]["state"] == "failed" {
                        ensure!(
                            status.body["data"]["logging"]["file"]["ioErrors"] == "1",
                            "first file failure was not counted separately"
                        );
                        ensure!(
                            status.body["data"]["storage"]["state"] == "ready",
                            "log failure corrupted storage health"
                        );
                        failed = true;
                        break;
                    }
                }
                ensure!(failed, "invalid rollover target did not latch file health");
                fs::remove_dir(&backup).context("restore rollover destination")?;
                let reply = item.request(request(&[])).await?;
                ensure!(reply.status == 200, "generation after log failure failed");
                item.record(reply.request_id()?).await?;
                let status = item.manager("/admin/v1/status").await?;
                ensure!(
                    status.body["data"]["logging"]["file"]["state"] == "failed"
                        && status.body["data"]["logging"]["file"]["ioErrors"] == "1",
                    "failed file writer retried after path repair"
                );
                Ok(())
            },
        )
        .await
}

fn record(id: &str, level: &str) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(&json!({"schemaVersion":1,"timestamp":"2026-10-16T00:00:00.000Z","level":level,"logId":id,"event":"requestFinished","requestId":"test-request"})).expect("fixed log fixture serializes");
    bytes.push(b'\n');
    bytes
}

async fn check_symlink(item: &super::fixture::Fixture, directory: &std::path::Path) -> Result<()> {
    let target = directory
        .parent()
        .context("fixture parent")?
        .join("outside-log.jsonl");
    fs::write(&target, record("outside-log", "info"))?;
    let link = directory.join("limen.jsonl.5");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&target, &link).context("create log symlink fixture")?;
    #[cfg(windows)]
    if let Err(error) = std::os::windows::fs::symlink_file(&target, &link) {
        if error.kind() == std::io::ErrorKind::PermissionDenied
            || error.raw_os_error() == Some(1314)
        {
            eprintln!(
                "log symlink case unavailable: Windows symlink privilege absent; non-regular directory rejection was verified"
            );
            return Ok(());
        }
        return Err(error).context("create log symlink fixture");
    }
    item.manager("/admin/v1/logs/files").await?.error(
        503,
        "storage_unavailable",
        "log reader followed a symbolic link",
    )?;
    fs::remove_file(link).context("remove log symlink fixture")?;
    Ok(())
}
