use anyhow::{ensure, Context, Result};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
    sync::atomic::{AtomicUsize, Ordering},
};
use tiny_http::{Request, Response, StatusCode};
static TRANSFERS: AtomicUsize = AtomicUsize::new(0);
struct Slot;
impl Drop for Slot {
    fn drop(&mut self) {
        TRANSFERS.fetch_sub(1, Ordering::AcqRel);
    }
}
pub fn byte_range(header: Option<&str>, len: u64) -> Result<Option<(u64, u64)>> {
    let Some(header) = header else {
        return Ok(None);
    };
    ensure!(len > 0, "Empty file has no byte range");
    let value = header
        .strip_prefix("bytes=")
        .context("Unsupported range unit")?;
    ensure!(
        !value.contains(','),
        "Multiple byte ranges are not supported"
    );
    let (a, b) = value.split_once('-').context("Malformed byte range")?;
    let (start, end) = if a.is_empty() {
        let count: u64 = b.parse()?;
        ensure!(count > 0, "Empty suffix range");
        (len.saturating_sub(count), len - 1)
    } else {
        let start: u64 = a.parse()?;
        let end = if b.is_empty() {
            len - 1
        } else {
            b.parse::<u64>()?.min(len - 1)
        };
        (start, end)
    };
    ensure!(start < len && start <= end, "Unsatisfiable byte range");
    Ok(Some((start, end)))
}
pub fn serve(req: Request, root: &Path, url: &str) {
    let previous = TRANSFERS.fetch_add(1, Ordering::AcqRel);
    if previous >= 16 {
        TRANSFERS.fetch_sub(1, Ordering::AcqRel);
        let _ = req.respond(crate::service::response(
            b"Too many audio transfers".to_vec(),
            "text/plain",
            503,
        ));
        return;
    }
    let _slot = Slot;
    let result: Result<(File, u64, bool)> = (|| {
        let relative = crate::service::unescape(&url[1..])?;
        let relative = Path::new(&relative);
        ensure!(
            relative
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_))),
            "Invalid export path"
        );
        let path = root.join(relative).canonicalize()?;
        ensure!(
            path.starts_with(root.join("exports").canonicalize()?),
            "Invalid export path"
        );
        let file = File::open(&path)?;
        let len = file.metadata()?.len();
        let wav = path.extension().is_some_and(|e| e == "wav");
        Ok((file, len, wav))
    })();
    let (mut file, len, wav) = match result {
        Ok(v) => v,
        Err(_) => {
            let _ = req.respond(crate::service::response(
                b"Not found".to_vec(),
                "text/plain",
                404,
            ));
            return;
        }
    };
    let range = req
        .headers()
        .iter()
        .find(|h| h.field.equiv("Range"))
        .map(|h| h.value.as_str());
    let range = match byte_range(range, len) {
        Ok(v) => v,
        Err(_) => {
            let _ = req.respond(
                Response::empty(StatusCode(416))
                    .with_header(crate::service::header(
                        "Content-Range",
                        &format!("bytes */{len}"),
                    ))
                    .with_header(crate::service::header("Accept-Ranges", "bytes")),
            );
            return;
        }
    };
    let (start, size, status) = if let Some((a, b)) = range {
        (a, b - a + 1, 206)
    } else {
        (0, len, 200)
    };
    if file.seek(SeekFrom::Start(start)).is_err() {
        let _ = req.respond(Response::empty(StatusCode(500)));
        return;
    }
    let mut headers = vec![
        crate::service::header(
            "Content-Type",
            if wav {
                "audio/wav"
            } else {
                "application/octet-stream"
            },
        ),
        crate::service::header("Accept-Ranges", "bytes"),
        crate::service::header("Cache-Control", "no-store"),
        crate::service::header("X-Content-Type-Options", "nosniff"),
    ];
    if status == 206 {
        headers.push(crate::service::header(
            "Content-Range",
            &format!("bytes {}-{}/{}", start, start + size - 1, len),
        ))
    }
    let _ = req.respond(
        Response::new(
            StatusCode(status),
            headers,
            file.take(size),
            Some(size as usize),
            None,
        )
        .with_chunked_threshold(usize::MAX),
    );
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn single_open_suffix_and_invalid_byte_ranges() -> Result<()> {
        assert_eq!(byte_range(Some("bytes=10-19"), 100)?, Some((10, 19)));
        assert_eq!(byte_range(Some("bytes=90-"), 100)?, Some((90, 99)));
        assert_eq!(byte_range(Some("bytes=-12"), 100)?, Some((88, 99)));
        assert_eq!(byte_range(Some("bytes=0-999"), 100)?, Some((0, 99)));
        for h in [
            "bytes=100-",
            "bytes=9-3",
            "bytes=-0",
            "bytes=0-1,3-4",
            "other=1-2",
        ] {
            assert!(byte_range(Some(h), 100).is_err())
        }
        Ok(())
    }
}
