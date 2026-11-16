//! Best-effort local screenshot sink. Called on a blocking worker by the transport.
use base64::Engine;
use serde_json::Value;
use std::{fs, io::Write, path::Path, time::SystemTime};

pub fn save_result(dir: &str, session: &str, result: &Value) {
    if dir.is_empty() {
        return;
    }
    for block in result["content"].as_array().into_iter().flatten() {
        if block["type"] != "image" {
            continue;
        }
        let outcome = (|| -> Result<(), Box<dyn std::error::Error>> {
            if session.is_empty()
                || !session
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            {
                return Err("invalid screenshot session component".into());
            }
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(block["data"].as_str().ok_or("missing image data")?)?;
            let millis = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)?
                .as_millis();
            let (year, month, day) = date((millis / 86_400_000) as i64);
            let path = Path::new(dir)
                .join("agentaccess/screenshots")
                .join(format!("year={year:04}/month={month:02}/day={day:02}"))
                .join(session);
            private_dirs(&path)?;
            // PROVISIONAL (not published by AWS): real JPEG extension, UTC epoch millis
            // plus collision counter, create_new atomic publication; Windows ACLs best effort.
            let ext = if block["mimeType"] == "image/jpeg" {
                "jpg"
            } else {
                "png"
            };
            for counter in 0u64.. {
                let target = path.join(format!("{millis}-{counter}.{ext}"));
                let mut options = fs::OpenOptions::new();
                options.write(true).create_new(true); // Never follows an existing final symlink.
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.mode(0o600);
                }
                match options.open(&target) {
                    Ok(mut file) => {
                        if let Err(e) = file.write_all(&bytes) {
                            let _ = fs::remove_file(target);
                            return Err(e.into());
                        }
                        return Ok(());
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(e) => return Err(e.into()),
                }
            }
            unreachable!()
        })();
        if let Err(e) = outcome {
            eprintln!("warning: screenshot save failed: {e}");
        }
    }
}
fn private_dirs(path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        private_dirs(parent)?;
    }
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => Ok(()),
        Ok(_) => Err(std::io::Error::other(
            "screenshot directory is not a real directory",
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            #[cfg(unix)]
            let mut builder = fs::DirBuilder::new();
            #[cfg(not(unix))]
            let builder = fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(path) {
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => private_dirs(path),
                other => other,
            }
        }
        Err(e) => Err(e),
    }
}
// Gregorian civil date from Unix days (400-year era decomposition).
fn date(days: i64) -> (i64, i64, i64) {
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    y += i64::from(m <= 2);
    (y, m, d)
}
#[cfg(test)]
mod tests {
    #[test]
    fn dates() {
        assert_eq!(super::date(0), (1970, 1, 1));
        assert_eq!(super::date(19782), (2024, 2, 29));
    }
}
